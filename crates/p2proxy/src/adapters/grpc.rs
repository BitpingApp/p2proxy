use std::sync::OnceLock;

use color_eyre::eyre::{Context, Result};
use proxy_core::config::Config;
use tonic::transport::{Channel, ClientTlsConfig};

/// The process-wide gRPC channel to the Bitping auth service, built once from
/// config (URL + derived TLS domain) and reused thereafter.
static CHANNEL: OnceLock<Channel> = OnceLock::new();

pub fn channel(config: &Config) -> Result<Channel> {
    if let Some(channel) = CHANNEL.get() {
        return Ok(channel.clone());
    }
    let channel = build(config)?;
    let _ = CHANNEL.set(channel.clone());
    Ok(channel)
}

fn build(config: &Config) -> Result<Channel> {
    if !config.grpc_url.starts_with("https://") {
        return Ok(Channel::builder(config.grpc_url.clone().try_into()?).connect_lazy());
    }
    let tls = ClientTlsConfig::new()
        .with_enabled_roots()
        .domain_name(config.grpc_domain());
    let channel = Channel::builder(config.grpc_url.clone().try_into()?)
        .tls_config(tls)
        .context("configuring TLS for gRPC")?
        .connect_lazy();
    Ok(channel)
}
