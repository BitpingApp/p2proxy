use std::collections::HashMap;
use std::io::Write;
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

    let config = Arc::new(
        Config::from_path(&cli.config)
            .with_context(|| format!("loading config from {}", cli.config))?,
    );

    install_logging(config.log_level.as_deref(), cli.no_ui)?;
    color_eyre::install()?;

    PrometheusBuilder::new()
        .with_http_listener(config.metrics_addr())
        .add_global_label("service", "p2proxy")
        .install()?;
    tracing::info!(metrics = %config.metrics_addr(), "metrics server running");

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
        let discovery = DiscoveryHandle::new(disc_tx.clone());
        tasks.spawn(async move {
            let result = tui::Ui::run_ui(events_rx, discovery, shutdown.clone(), config).await;
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

    // Run until shutdown is requested (Ctrl+C / TUI quit) or a task ends early.
    let mut first_error: Option<color_eyre::Report> = None;
    tokio::select! {
        _ = shutdown.cancelled() => {}
        joined = tasks.join_next() => {
            match joined {
                Some(Ok(Err(e))) => first_error = Some(e),
                Some(Err(join_err)) => {
                    first_error = Some(color_eyre::eyre::eyre!("task panicked: {join_err}"));
                }
                _ => {}
            }
            shutdown.cancel();
        }
    }

    if cli.no_ui {
        tracing::info!("shutting down…");
    }

    // Drain the rest with a hard cap so cleanup can never hang the process.
    let drained = tokio::time::timeout(SHUTDOWN_GRACE, async {
        let mut err = None;
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Err(e)) if err.is_none() => err = Some(e),
                Err(join_err) if err.is_none() => {
                    err = Some(color_eyre::eyre::eyre!("task panicked: {join_err}"));
                }
                _ => {}
            }
        }
        err
    })
    .await;

    let Ok(drain_error) = drained else {
        shutdown_note(cli.no_ui, "cleanup timed out — forcing exit");
        std::process::exit(i32::from(first_error.is_some()));
    };

    shutdown_note(cli.no_ui, "shutdown complete");
    match first_error.or(drain_error) {
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

    let boot = bootstrap::bootstrap((**keypair).clone(), config, events_tx).await?;

    Ok((boot, token))
}

const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

fn spawn_ctrl_c_handler(shutdown: CancellationToken) {
    tokio::spawn(async move {
        // First Ctrl+C: graceful. Second: force-quit, so a slow cleanup is
        // always escapable instead of looking like a hang.
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Ctrl+C received — shutting down (press Ctrl+C again to force quit)");
            shutdown.cancel();
        }
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = writeln!(std::io::stderr(), "p2proxy: forced shutdown");
            std::process::exit(130);
        }
    });
}

/// Surface shutdown progress in whichever mode we're in: tracing in headless
/// (it reaches stdout), direct stderr in TUI mode (where tracing is captured by
/// the log pane and the terminal has already been restored by this point).
fn shutdown_note(no_ui: bool, msg: &str) {
    if no_ui {
        tracing::info!("{msg}");
    } else {
        let _ = writeln!(std::io::stderr(), "p2proxy: {msg}");
    }
}

fn install_logging(log_level: Option<&str>, no_ui: bool) -> Result<()> {
    let default_directive: tracing_subscriber::filter::Directive = log_level
        .and_then(|level| level.parse().ok())
        .unwrap_or_else(|| LevelFilter::INFO.into());
    let log_filter = || {
        EnvFilter::builder()
            .with_default_directive(default_directive.clone())
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
