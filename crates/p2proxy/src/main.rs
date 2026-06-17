use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use clap::Parser;
use color_eyre::eyre::{Context, Result};
use libp2p::PeerId;
use libp2p::identity::Keypair;
use metrics_exporter_prometheus::PrometheusBuilder;
use proxy_core::config::Config;
use proxy_core::events::Events;
use proxy_core::ports::Authenticator;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

mod adapters;
mod args;
mod runtime;
mod tui;
mod utils;

use adapters::channel_sink::ChannelSink;
use adapters::file_sticky::{FileStickyStore, default_sticky_path};
use adapters::grpc_auth::GrpcAuth;
use adapters::keypair_identity::{KeypairIdentity, load_or_generate_keypair};
use adapters::tokio_clock::TokioClock;
use args::Cli;
use runtime::Context as AppContext;
use runtime::Runtime;
use runtime::discovery::{DestinationHandle, DiscoveryActor, DiscoveryEvent, DiscoveryHandle};
use runtime::network::{NetworkActor, NetworkCommand, NetworkHandle, bootstrap};
use runtime::session::{SessionContext, SessionSupervisor};
use runtime::stream_manager::PeerStreamManager;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    install_logging(cli.no_ui)?;
    color_eyre::install()?;

    let config = Arc::new(
        Config::from_path(&cli.config)
            .with_context(|| format!("loading config from {}", cli.config))?,
    );

    PrometheusBuilder::new()
        .with_http_listener(config.metrics_addr)
        .add_global_label("service", "p2proxy")
        .install()?;
    tracing::info!(metrics = %config.metrics_addr, "metrics server running");

    let keypair = Arc::new(load_or_generate_keypair(std::path::Path::new(
        &config.keypair_path,
    )));

    let (events_tx, events_rx) = mpsc::channel::<Events>(200);
    let (net_tx, net_rx) = mpsc::channel::<NetworkCommand>(100);
    let (disc_tx, disc_rx) = mpsc::channel::<DiscoveryEvent>(100);

    let shutdown = CancellationToken::new();
    spawn_ctrl_c_handler(shutdown.clone());

    let mut tasks: JoinSet<Result<()>> = JoinSet::new();

    if cli.no_ui {
        tracing::info!("running headless (--no-ui)");
        drop(events_rx);
    } else {
        let shutdown = shutdown.clone();
        let config = config.clone();
        tasks.spawn(async move {
            let result = tui::Ui::run_ui(events_rx, shutdown.clone(), config).await;
            shutdown.cancel();
            result
        });
    }

    // Auth + bootstrap must finish before the actors can run. A failure here
    // tears the already-spawned TUI down cleanly before returning.
    let (boot, token) = match start_network(&config, &keypair, &events_tx).await {
        Ok(started) => started,
        Err(e) => {
            shutdown.cancel();
            while tasks.join_next().await.is_some() {}
            return Err(e);
        }
    };

    let mut destinations: HashMap<u16, DestinationHandle> = HashMap::new();
    for server in &config.servers {
        destinations.insert(server.port, Arc::new(ArcSwap::from_pointee(None::<PeerId>)));
    }

    let max_concurrent = config
        .servers
        .iter()
        .map(|s| s.pool.max_total)
        .max()
        .unwrap_or(30);
    let open_timeout = config
        .servers
        .iter()
        .map(|s| s.pool.open_timeout_secs)
        .min()
        .unwrap_or(20);
    let streams = Arc::new(PeerStreamManager::new(
        boot.stream_control,
        max_concurrent,
        Duration::from_secs(open_timeout),
    ));

    let ctx = AppContext {
        config: config.clone(),
        keypair,
        token,
        relay_peer_id: boot.relay_peer_id,
        relay_address: boot.relay_address,
        bootstrap_peer_id: boot.bootstrap_peer_id,
        bootstrap_address: boot.bootstrap_address,
        client: boot.client,
        events: ChannelSink::new(events_tx),
        network: NetworkHandle::new(net_tx),
        discovery: DiscoveryHandle::new(disc_tx),
        streams,
        clock: TokioClock,
    };

    let network = NetworkActor::new(boot.swarm);
    let discovery = DiscoveryActor::new(
        FileStickyStore::load(default_sticky_path()),
        destinations.clone(),
    );
    Runtime::spawn(
        ctx.clone(),
        network,
        net_rx,
        discovery,
        disc_rx,
        shutdown.clone(),
        &mut tasks,
    );

    for server in &config.servers {
        let session = SessionContext {
            port: server.port,
            keypair: ctx.keypair.clone(),
            token: ctx.token.clone(),
            destination: destinations[&server.port].clone(),
            discovery: ctx.discovery.clone(),
            streams: ctx.streams.clone(),
            net: ctx.network.clone(),
            events: ctx.events.clone(),
        };
        SessionSupervisor::spawn(session)
            .await
            .with_context(|| format!("binding SOCKS listener on :{}", server.port))?;
        ctx.discovery.discover_for(server.port).await;
    }

    let mut first_error: Option<color_eyre::Report> = None;
    while let Some(joined) = tasks.join_next().await {
        let outcome = match joined {
            Ok(result) => result,
            Err(join_err) => Err(color_eyre::eyre::eyre!("task panicked: {join_err}")),
        };
        if let Err(e) = outcome
            && first_error.is_none()
        {
            first_error = Some(e);
            shutdown.cancel();
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Authenticate against the hub and bootstrap the swarm.
async fn start_network(
    config: &Arc<Config>,
    keypair: &Arc<Keypair>,
    events_tx: &mpsc::Sender<Events>,
) -> Result<(bootstrap::Bootstrapped, String)> {
    let identity = KeypairIdentity::new(keypair.clone());
    let auth = GrpcAuth::new(
        adapters::grpc::channel(config)?,
        config.bitping_api_key.to_string(),
        identity,
    );
    let token = auth.federated_token().await.context("federated auth")?;

    let boot = bootstrap::bootstrap(
        (**keypair).clone(),
        config.port,
        config.bootstrap.clone(),
        events_tx,
    )
    .await?;

    Ok((boot, token))
}

fn spawn_ctrl_c_handler(shutdown: CancellationToken) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Ctrl+C received — initiating graceful shutdown");
            shutdown.cancel();
        }
    });
}

fn install_logging(no_ui: bool) -> Result<()> {
    let log_filter = || {
        EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .from_env_lossy()
    };

    if no_ui {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::Layer::default()
                    .compact()
                    .pretty()
                    .with_file(true)
                    .with_filter(log_filter()),
            )
            .with(ErrorLayer::default())
            .init();
        return Ok(());
    }

    std::panic::set_hook(Box::new(|panic_info| {
        let _ = crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
        better_panic::Settings::auto()
            .most_recent_first(false)
            .lineno_suffix(true)
            .create_panic_handler()(panic_info);
    }));

    tui_logger::init_logger(tui_logger::LevelFilter::Trace)
        .map_err(|e| color_eyre::eyre::eyre!("failed to init tui_logger: {e}"))?;
    tui_logger::set_default_level(tui_logger::LevelFilter::Trace);
    let log_file_path = tui_components::logs::install_file_mirror("p2proxy");
    tracing::info!(path = %log_file_path.display(), "mirroring logs to file");
    tracing_subscriber::registry()
        .with(tui_logger::tracing_subscriber_layer().with_filter(log_filter()))
        .with(ErrorLayer::default())
        .init();
    Ok(())
}
