use std::{
    borrow::Cow,
    fmt::Display,
};

use color_eyre::eyre;
use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use human_bandwidth::re::bandwidth::Bandwidth;
use libp2p::Multiaddr;
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
#[derive(Serialize, Deserialize, Debug, Clone)]
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

    /// Timeout for acquiring semaphore permit in seconds (rate limiting)
    #[serde(default)]
    pub semaphore_timeout_secs: Option<u64>,

    /// Number of retry attempts for failed requests
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Timeout for peer health checks in seconds
    #[serde(default = "default_health_check_timeout_secs")]
    pub health_check_timeout_secs: u64,

    /// Maximum error rate before triggering failover (0.0-1.0)
    #[serde(default = "default_max_error_rate")]
    pub max_error_rate: f64,
}

// Manual implementations of Hash, Eq, PartialEq, Ord, PartialOrd for PoolConfigOptions
// f64 doesn't implement these traits, so we convert to bits for hashing/comparison
impl std::hash::Hash for PoolConfigOptions {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.enabled.hash(state);
        self.min_idle.hash(state);
        self.max_total.hash(state);
        self.idle_timeout_secs.hash(state);
        self.open_timeout_secs.hash(state);
        self.semaphore_timeout_secs.hash(state);
        self.max_retries.hash(state);
        self.health_check_timeout_secs.hash(state);
        self.max_error_rate.to_bits().hash(state); // Convert f64 to u64 for hashing
    }
}

impl PartialEq for PoolConfigOptions {
    fn eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.min_idle == other.min_idle
            && self.max_total == other.max_total
            && self.idle_timeout_secs == other.idle_timeout_secs
            && self.open_timeout_secs == other.open_timeout_secs
            && self.semaphore_timeout_secs == other.semaphore_timeout_secs
            && self.max_retries == other.max_retries
            && self.health_check_timeout_secs == other.health_check_timeout_secs
            && self.max_error_rate.to_bits() == other.max_error_rate.to_bits()
    }
}

impl Eq for PoolConfigOptions {}

impl PartialOrd for PoolConfigOptions {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PoolConfigOptions {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.enabled
            .cmp(&other.enabled)
            .then_with(|| self.min_idle.cmp(&other.min_idle))
            .then_with(|| self.max_total.cmp(&other.max_total))
            .then_with(|| self.idle_timeout_secs.cmp(&other.idle_timeout_secs))
            .then_with(|| self.open_timeout_secs.cmp(&other.open_timeout_secs))
            .then_with(|| self.semaphore_timeout_secs.cmp(&other.semaphore_timeout_secs))
            .then_with(|| self.max_retries.cmp(&other.max_retries))
            .then_with(|| self.health_check_timeout_secs.cmp(&other.health_check_timeout_secs))
            .then_with(|| self.max_error_rate.to_bits().cmp(&other.max_error_rate.to_bits()))
    }
}

impl Default for PoolConfigOptions {
    fn default() -> Self {
        Self {
            enabled: default_pool_enabled(),
            min_idle: default_min_idle(),
            max_total: default_max_total(),
            idle_timeout_secs: default_idle_timeout_secs(),
            open_timeout_secs: default_open_timeout_secs(),
            semaphore_timeout_secs: None,  // Defaults to None for backward compatibility
            max_retries: default_max_retries(),
            health_check_timeout_secs: default_health_check_timeout_secs(),
            max_error_rate: default_max_error_rate(),
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
    30
}

fn default_idle_timeout_secs() -> u64 {
    60
}

fn default_open_timeout_secs() -> u64 {
    20
}

fn default_max_retries() -> u32 {
    3
}

fn default_health_check_timeout_secs() -> u64 {
    5
}

fn default_max_error_rate() -> f64 {
    0.15
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
