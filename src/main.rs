use std::sync::LazyLock;

use color_eyre::eyre::{Context, Result};
use config::Config;
use metrics_exporter_prometheus::PrometheusBuilder;
use swarm::ProxyNetwork;
use tokio::task::JoinSet;
use tonic::transport::{Channel, ClientTlsConfig};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod config;
mod events;
mod proxy_protocols;
mod swarm;
mod ui;
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

    // Check if we're running in TUI mode
    let use_tui = !CONFIG.disable_ui; // You'll need to determine this based on your application's needs

    if use_tui {
        // Only use the TUI logger
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tui_logger::TuiTracingSubscriberLayer)
            .init();
        tui_logger::init_logger(tui_logger::LevelFilter::Info)?;
    } else {
        // Use both TUI logger and standard output
        let fmt = tracing_subscriber::fmt::Layer::default()
            .compact()
            .pretty()
            .with_file(true);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(tui_logger::TuiTracingSubscriberLayer)
            .with(fmt)
            .init();
    }

    color_eyre::install()?;

    let builder = PrometheusBuilder::new();
    builder.install()?;

    let (tx, rx) = tokio::sync::mpsc::channel(100);

    let mut join_set = JoinSet::new();

    if use_tui {
        let _ = join_set.spawn(ui::Ui::run_ui(rx));
    }

    let mut proxy_future = ProxyNetwork::with_authentication()
        .await?
        .with_swarm(tx)
        .await?;

    for server in CONFIG.servers.iter() {
        proxy_future.configure_server(server).await?;
    }

    let _ = join_set.spawn(async move { proxy_future.drive_network().await });

    while let Some(result) = join_set.join_next().await {
        result??;
    }

    // Wait for both to complete

    Ok(())
}
