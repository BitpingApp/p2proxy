use std::{
    borrow::Cow,
    fmt::{write, Display},
    sync::Arc,
};

use color_eyre::eyre;
use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use human_bandwidth::re::bandwidth::Bandwidth;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub struct Config {
    pub servers: Vec<Server>,
    pub port: u16,
    pub bitping_api_key: Cow<'static, str>,
}

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub enum ProxyProtocols {
    Socks5,
}

/// Pool configuration options for stream pooling
#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub struct PoolConfigOptions {
    /// Whether pooling is enabled (defaults to true)
    #[serde(default = "default_pool_enabled")]
    pub enabled: bool,

    /// Minimum number of idle streams to maintain per peer
    #[serde(default = "default_min_idle")]
    pub min_idle: usize,

    /// Maximum total streams (idle + active) per peer
    #[serde(default = "default_max_total")]
    pub max_total: usize,

    /// Maximum idle duration in seconds before recycling
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,

    /// Timeout for opening a new stream in seconds
    #[serde(default = "default_open_timeout_secs")]
    pub open_timeout_secs: u64,
}

impl Default for PoolConfigOptions {
    fn default() -> Self {
        Self {
            enabled: default_pool_enabled(),
            min_idle: default_min_idle(),
            max_total: default_max_total(),
            idle_timeout_secs: default_idle_timeout_secs(),
            open_timeout_secs: default_open_timeout_secs(),
        }
    }
}

fn default_pool_enabled() -> bool {
    true
}

fn default_min_idle() -> usize {
    5
}

fn default_max_total() -> usize {
    20
}

fn default_idle_timeout_secs() -> u64 {
    60
}

fn default_open_timeout_secs() -> u64 {
    10
}

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub struct Server {
    pub protocol: ProxyProtocols,
    pub port: u16,

    #[serde(flatten)]
    pub peer_options: ServerPeerOptions,

    #[serde(default)]
    pub pool: PoolConfigOptions,
}

#[derive(Serialize, Deserialize, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub struct ServerPeerOptions {
    // TODO: Eventually replace this with some more options.
    pub destination_peer: Option<Multiaddr>,
    pub country: Option<String>,
    #[serde(default = "default_min_bandwith")]
    #[serde(with = "human_bandwidth::serde")]
    pub min_bandwidth: Bandwidth,
}

fn default_min_bandwith() -> Bandwidth {
    Bandwidth::from_mbps(50)
}

impl Display for ServerPeerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let opt_string = if let Some(p) = &self.destination_peer {
            format!("Destination Peer: {p:#?}")
        } else if let Some(c) = &self.country {
            format!("Country: {c}")
        } else {
            format!("Unknown")
        };

        write!(f, "{}", opt_string)
    }
}

impl Config {
    pub fn new() -> eyre::Result<Config> {
        Ok(Figment::new()
            .merge(Yaml::file("Config.yaml"))
            .merge(Env::raw())
            .extract()?)
    }
}
