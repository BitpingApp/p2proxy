use std::{
    net::Ipv4Addr,
    sync::{Arc, LazyLock},
};

use color_eyre::eyre::{Context, Result};
use metrics_exporter_prometheus::PrometheusBuilder;
use models::config::Config;
use models::{CounterClient, CounterServerSharedMut, ServerContainer, events::Events};
use remoc::{codec::Codec, rch, rtc::ServerSharedMut};
use swarm::ProxyNetwork;
use tokio::{net::TcpListener, sync::RwLock, task::JoinSet};
use tonic::transport::{Channel, ClientTlsConfig};
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod proxy_protocols;
mod stream_pool;
mod swarm;
mod utils;

static GRPC_CHANNEL: LazyLock<Channel> = LazyLock::new(|| {
    get_grpc_channel("https://grpc.bitping.com".into(), "grpc.bitping.com".into())
        .expect("Failed to resolve GRPC Channel")
});

pub fn get_grpc_channel(grpc_hub_url: String, grpc_hub_domain: String) -> Result<Channel> {
    let channel_config = if grpc_hub_url.starts_with("https://") {
        let tls = ClientTlsConfig::new()
            .with_enabled_roots()
            .domain_name(grpc_hub_domain);
        Channel::builder(grpc_hub_url.try_into()?)
            .tls_config(tls)
            .context("Error configuring TLS for GRPC")?
    } else {
        Channel::builder(grpc_hub_url.try_into()?)
    };

    Ok(channel_config.connect_lazy())
}

static CONFIG: LazyLock<Config> =
    LazyLock::new(|| Config::new().expect("Cannot initialise config"));

#[tokio::main]
async fn main() -> Result<()> {
    std::panic::set_hook(Box::new(|panic_info| {
        crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen).unwrap();
        crossterm::terminal::disable_raw_mode().unwrap();
        better_panic::Settings::auto()
            .most_recent_first(false)
            .lineno_suffix(true)
            .create_panic_handler()(panic_info);
    }));

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let fmt = tracing_subscriber::fmt::Layer::default()
        .compact()
        .pretty()
        .with_file(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt)
        .with(ErrorLayer::default())
        .init();

    color_eyre::install()?;

    // Setup prometheus metrics exporter
    let builder = PrometheusBuilder::new();
    builder
        .with_http_listener(([0, 0, 0, 0], 9091))
        .add_global_label("service", "p2proxy")
        .install()?;

    tracing::info!("Metrics server running on http://0.0.0.0:9091/metrics");

    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let mut join_set = JoinSet::new();

    let mut proxy_future = ProxyNetwork::with_authentication()
        .await?
        .with_swarm(tx)
        .await?;

    for server in CONFIG.servers.iter() {
        proxy_future.configure_server(server).await?;
    }

    let server_state = Arc::new(RwLock::new(ServerContainer::new(CONFIG.servers.clone())));
    let _ = join_set.spawn(proxy_future.drive_network(server_state.clone()));
    let _ = join_set.spawn(start_server(server_state.clone()));
    let _ = join_set.spawn(handle_swarm_events(rx, server_state.clone()));

    while let Some(result) = join_set.join_next().await {
        result??;
    }

    // Wait for both to complete

    Ok(())
}

async fn handle_swarm_events(
    mut rx: tokio::sync::mpsc::Receiver<Events>,
    server_state: Arc<RwLock<ServerContainer>>,
) -> Result<()> {
    while let Some(event) = rx.recv().await {
        let mut state = server_state.write().await;
        state.handle_event(event).await;
    }
    Ok(())
}

const TCP_PORT: u16 = 9876;
const MAX_CONSECUTIVE_ERRORS: u32 = 10;

async fn start_server(server_state: Arc<RwLock<ServerContainer>>) -> Result<()> {
    use remoc::ConnectExt;
    use std::time::{Duration, Instant};

    println!("Listening on port {}. Press Ctrl+C to exit.", TCP_PORT);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, TCP_PORT)).await?;

    let mut consecutive_errors = 0;
    let mut last_success = Instant::now();

    loop {
        // Accept an incoming TCP connection with error handling
        let (socket, addr) = match listener.accept().await {
            Ok(conn) => {
                consecutive_errors = 0;  // Reset on success
                last_success = Instant::now();
                metrics::gauge!("p2proxy_rpc_consecutive_accept_errors").set(0.0);
                conn
            }
            Err(e) => {
                consecutive_errors += 1;
                metrics::counter!("p2proxy_rpc_accept_errors_total").increment(1);
                metrics::gauge!("p2proxy_rpc_consecutive_accept_errors")
                    .set(consecutive_errors as f64);

                if consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                    tracing::error!(
                        "Too many consecutive accept errors ({}) - last success {:?} ago. \
                         This may indicate system-level issues (file descriptors, permissions). \
                         Error: {}",
                        consecutive_errors,
                        last_success.elapsed(),
                        e
                    );
                    // Longer backoff when many consecutive errors
                    tokio::time::sleep(Duration::from_secs(1)).await;
                } else {
                    tracing::error!("Failed to accept RPC connection: {}", e);
                    // Brief backoff for transient errors
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                continue;
            }
        };

        let (socket_rx, socket_tx) = socket.into_split();
        tracing::debug!("Accepted RPC connection from {}", addr);
        let counter_obj = server_state.clone();

        // Spawn a task for each incoming connection
        tokio::spawn(async move {
            let (server, client) =
                CounterServerSharedMut::<_, remoc::codec::Postcard>::new(counter_obj, 1);

            // Handle remoc connection with error handling (no panic)
            match remoc::Connect::io(remoc::Cfg::default(), socket_rx, socket_tx)
                .provide(client)
                .await
            {
                Ok(_connection) => {
                    tracing::info!("Established RPC connection from {}", addr);
                    metrics::counter!("p2proxy_rpc_connections_total").increment(1);
                    metrics::gauge!("p2proxy_rpc_active_connections").increment(1.0);

                    // Serve the connection
                    if let Err(e) = server.serve(true).await {
                        tracing::warn!("RPC server error for {}: {}", addr, e);
                        metrics::counter!("p2proxy_rpc_serve_errors_total").increment(1);
                    }

                    metrics::gauge!("p2proxy_rpc_active_connections").decrement(1.0);
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to establish remoc connection from {}. \
                         This could indicate malformed client or incompatible codec. \
                         Error: {}",
                        addr,
                        e
                    );
                    metrics::counter!("p2proxy_rpc_connection_errors_total").increment(1);
                }
            }
        });
    }
}
