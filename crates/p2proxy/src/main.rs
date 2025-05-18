use std::{
    net::Ipv4Addr,
    sync::{Arc, LazyLock},
};

use color_eyre::eyre::{Context, Result};
use config::Config;
use metrics_exporter_prometheus::PrometheusBuilder;
use models::{CounterClient, CounterObj, CounterServerSharedMut};
use remoc::{codec::Codec, rtc::ServerSharedMut};
use swarm::ProxyNetwork;
use tokio::{net::TcpListener, sync::RwLock, task::JoinSet};
use tonic::transport::{Channel, ClientTlsConfig};
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod config;
mod proxy_protocols;
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

    let builder = PrometheusBuilder::new();
    builder.install()?;

    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let mut join_set = JoinSet::new();

    let mut proxy_future = ProxyNetwork::with_authentication()
        .await?
        .with_swarm(tx)
        .await?;

    for server in CONFIG.servers.iter() {
        proxy_future.configure_server(server).await?;
    }

    let _ = join_set.spawn(async move { proxy_future.drive_network().await });
    let _ = join_set.spawn(start_server());

    while let Some(result) = join_set.join_next().await {
        result??;
    }

    // Wait for both to complete

    Ok(())
}

const TCP_PORT: u16 = 9876;
async fn start_server() -> Result<()> {
    use remoc::ConnectExt;
    // Create a counter object that will be shared between all clients.
    // You could also create one counter object per connection.
    let counter_obj = Arc::new(RwLock::new(CounterObj::default()));

    // Listen to TCP connections using Tokio.
    // In reality you would probably use TLS or WebSockets over HTTPS.
    println!("Listening on port {}. Press Ctrl+C to exit.", TCP_PORT);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, TCP_PORT)).await?;

    loop {
        // Accept an incoming TCP connection.
        let (socket, addr) = listener.accept().await.unwrap();
        let (socket_rx, socket_tx) = socket.into_split();
        println!("Accepted connection from {}", addr);

        // Create a new shared reference to the counter object.
        let counter_obj = counter_obj.clone();

        // Spawn a task for each incoming connection.
        tokio::spawn(async move {
            // Create a server proxy and client for the accepted connection.
            //
            // The server proxy executes all incoming method calls on the shared counter_obj
            // with a request queue length of 1.
            //
            // Current limitations of the Rust compiler require that we explicitly
            // specify the codec.
            let (server, client) =
                CounterServerSharedMut::<_, remoc::codec::Postcard>::new(counter_obj, 1);

            remoc::Connect::io(remoc::Cfg::default(), socket_rx, socket_tx)
                .provide(client)
                .await
                .unwrap();

            tracing::info!("Serving database connection {}", addr);
            server.serve(true).await
        });
    }
}
