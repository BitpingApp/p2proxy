use std::sync::{Arc, LazyLock};

use color_eyre::eyre::{Context, Result};
use metrics_exporter_prometheus::PrometheusBuilder;
use models::config::Config;
use models::events::Events;
use models::ServerContainer;
use swarm::ProxyNetwork;
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tonic::transport::{Channel, ClientTlsConfig};
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

mod discovery;
mod proxy_protocols;
mod stream_pool;
mod swarm;
mod tui;
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

/// Resolved CLI arguments. Populated from `parse_args` exactly once at startup
/// so the rest of the code can read it via `CONFIG` / `ui_disabled` without
/// re-parsing.
struct CliArgs {
    /// Path to `Config.yaml`. From `--config <path>` / `-c <path>` /
    /// `P2PROXY_CONFIG=<path>`; defaults to `Config.yaml` in CWD.
    config_path: String,
    /// `--no-ui` / `NO_UI=true` — skip the ratatui rendering loop.
    no_ui: bool,
}

fn parse_args() -> CliArgs {
    let mut args = std::env::args().skip(1);
    let mut config_path: Option<String> = None;
    let mut no_ui = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-ui" => no_ui = true,
            "--config" | "-c" => {
                config_path = args.next();
                if config_path.is_none() {
                    eprintln!("error: --config requires a path argument");
                    std::process::exit(2);
                }
            }
            other if other.starts_with("--config=") => {
                config_path = Some(other.trim_start_matches("--config=").to_string());
            }
            "-h" | "--help" => {
                println!(
                    "p2proxy — Bitping P2P proxy daemon\n\nUSAGE:\n  p2proxy [OPTIONS]\n\nOPTIONS:\n  -c, --config <path>   Path to Config.yaml (default: ./Config.yaml; env: P2PROXY_CONFIG)\n      --no-ui           Run headless without the TUI (env: NO_UI=true)\n  -h, --help            Print this message\n      --version         Print version"
                );
                std::process::exit(0);
            }
            "--version" => {
                println!("p2proxy {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {
                // Forward-compat: unknown args are tolerated, in case a future
                // subcommand or wrapper passes flags this binary doesn't know.
                tracing::debug!(?arg, "ignoring unrecognised CLI argument");
            }
        }
    }

    let config_path = config_path
        .or_else(|| std::env::var("P2PROXY_CONFIG").ok())
        .unwrap_or_else(|| "Config.yaml".to_string());

    let no_ui_env = matches!(
        std::env::var("NO_UI").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    );

    CliArgs {
        config_path,
        no_ui: no_ui || no_ui_env,
    }
}

static CLI: LazyLock<CliArgs> = LazyLock::new(parse_args);

static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    Config::from_path(&CLI.config_path)
        .unwrap_or_else(|e| panic!("Cannot load config from {}: {e}", CLI.config_path))
});

#[tokio::main]
async fn main() -> Result<()> {
    let no_ui = CLI.no_ui;

    // The TUI takes over stderr/stdout when rendering, so we only install the
    // alt-screen panic recovery hook when the UI is actually running. In
    // headless mode let color_eyre handle panic printing normally.
    if !no_ui {
        std::panic::set_hook(Box::new(|panic_info| {
            let _ = crossterm::execute!(
                std::io::stderr(),
                crossterm::terminal::LeaveAlternateScreen
            );
            let _ = crossterm::terminal::disable_raw_mode();
            better_panic::Settings::auto()
                .most_recent_first(false)
                .lineno_suffix(true)
                .create_panic_handler()(panic_info);
        }));
    }

    // Logging sink depends on whether the TUI is rendering. In TUI mode the
    // fmt layer would scribble straight onto the alt-screen because both
    // ratatui and tracing-fmt write to stderr; instead we feed tui-logger's
    // ring buffer and let `render_logs_tab` draw it inside its own pane.
    // Headless mode keeps the original pretty fmt layer.
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
    } else {
        // tui-logger admits TRACE both at ring-buffer and display level so
        // EnvFilter is the only gate — otherwise `RUST_LOG=debug` would be
        // silently dropped by tui-logger's internal display filter.
        tui_logger::init_logger(tui_logger::LevelFilter::Trace)
            .map_err(|e| color_eyre::eyre::eyre!("Failed to init tui_logger: {e}"))?;
        tui_logger::set_default_level(tui_logger::LevelFilter::Trace);
        // Mirror every log line to a per-run file via the shared
        // tui-components helper — same setup bitpingd uses. The path is
        // discoverable via `tui_components::logs::log_file_path()`, and
        // the 'e' key in the TUI copies the file to the clipboard.
        let log_file_path = tui_components::logs::install_file_mirror("p2proxy");
        tracing::info!(path = %log_file_path.display(), "mirroring logs to file");
        tracing_subscriber::registry()
            .with(tui_logger::tracing_subscriber_layer().with_filter(log_filter()))
            .with(ErrorLayer::default())
            .init();
    }

    color_eyre::install()?;

    // Prometheus metrics — independent of UI mode.
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9091))
        .add_global_label("service", "p2proxy")
        .install()?;

    tracing::info!("Metrics server running on http://0.0.0.0:9091/metrics");

    // Swarm event channel — ProxyNetwork emits, two consumers:
    //   1. handle_swarm_events: mutates the shared ServerContainer state.
    //   2. The TUI (when not --no-ui): renders changes.
    //
    // Single mpsc for the swarm-side producer; handle_swarm_events fans out
    // to a second mpsc for the TUI. When --no-ui is set the second receiver
    // is dropped immediately so try_send becomes a cheap no-op.
    let (swarm_tx, swarm_rx) = tokio::sync::mpsc::channel::<Events>(100);
    let (tui_tx, tui_rx) = tokio::sync::mpsc::channel::<Events>(100);

    let mut join_set = JoinSet::new();

    // One cancellation token threaded through every long-running task so
    // Ctrl+C (or the TUI's 'q' key) triggers a clean libp2p disconnect on
    // its way out. Without this the OS just slams the TCP socket shut,
    // the hub keeps the stale session/reservation alive for its full
    // liveness-timeout window, and restarting p2proxy inside that window
    // produces the "Failed to connect with any peer after 20 attempts"
    // failure because FindNodes returns peers still pinned to the dead
    // session.
    let shutdown = tokio_util::sync::CancellationToken::new();

    // Ctrl+C → flip the cancel token. tokio::signal::ctrl_c handles both
    // SIGINT and (on Windows) Ctrl+Break.
    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            if let Ok(()) = tokio::signal::ctrl_c().await {
                tracing::info!("Ctrl+C received — initiating graceful shutdown");
                shutdown.cancel();
            }
        });
    }

    let server_state = Arc::new(RwLock::new(ServerContainer::new(CONFIG.servers.clone())));

    // Spawn the TUI BEFORE we authenticate against the hub or run peer
    // discovery so the user sees the dashboard (and the logs pane) the
    // instant the binary starts — otherwise startup looks frozen for the
    // 5–200s the FindNodes retry loop can run for, and there's no UI
    // available to even read the failure logs that would tell them what
    // to tweak in Config.yaml.
    if no_ui {
        tracing::info!("Running headless (--no-ui)");
        drop(tui_rx);
    } else {
        let tui_shutdown = shutdown.clone();
        join_set.spawn(async move {
            let result = tui::Ui::run_ui(tui_rx, tui_shutdown.clone()).await;
            // TUI returning (clean 'q' or panic) means the user wants out —
            // propagate the cancel so drive_network can disconnect cleanly.
            tui_shutdown.cancel();
            result
        });
    }

    join_set.spawn(handle_swarm_events(
        swarm_rx,
        server_state.clone(),
        tui_tx,
        shutdown.clone(),
    ));

    // The whole hub-handshake + per-server peer-lookup + steady-state event
    // loop runs as one background task. Failures during the configure
    // phase propagate up through join_set.join_next() and short-circuit
    // main, same as before — just now with the TUI already mounted to
    // render the logs.
    {
        let shutdown = shutdown.clone();
        let server_state = server_state.clone();
        join_set.spawn(async move {
            let mut proxy_future = ProxyNetwork::with_authentication()
                .await?
                .with_swarm(swarm_tx, no_ui)
                .await?;

            for server in CONFIG.servers.iter() {
                proxy_future.configure_server(server, &shutdown).await?;
            }

            proxy_future.drive_network(server_state, shutdown).await
        });
    }

    // If any task errors (e.g. drive_network returning "Failed to connect
    // with any peer after 20 attempts"), we MUST trigger shutdown so the
    // TUI gets the chance to leave the alt-screen and restore the terminal
    // before color-eyre prints the report. Without this, the error bleeds
    // straight onto the alt-screen on top of the TUI and the terminal
    // stays in raw mode after exit.
    let mut first_error: Option<color_eyre::Report> = None;
    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_error.is_none() {
                    first_error = Some(e);
                    shutdown.cancel();
                }
            }
            Err(join_err) => {
                if first_error.is_none() {
                    first_error = Some(color_eyre::eyre::eyre!("task panicked: {join_err}"));
                    shutdown.cancel();
                }
            }
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

async fn handle_swarm_events(
    mut rx: tokio::sync::mpsc::Receiver<Events>,
    server_state: Arc<RwLock<ServerContainer>>,
    tui_tx: tokio::sync::mpsc::Sender<Events>,
    shutdown: tokio_util::sync::CancellationToken,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            maybe_event = rx.recv() => {
                let Some(event) = maybe_event else { return Ok(()) };
                // Mirror to the TUI before mutating state so the renderer sees
                // the event the same instant the state changes. try_send
                // errors are ignored — when --no-ui is set the receiver was
                // dropped immediately and every send will fail; that's the
                // intended fast path.
                let _ = tui_tx.try_send(event.clone());
                let mut state = server_state.write().await;
                state.handle_event(event).await;
            }
        }
    }
}
