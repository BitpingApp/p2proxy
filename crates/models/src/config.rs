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
    /// libp2p multiaddr of the bootstrap hub. Defaults to Bitping's
    /// production hub (`/dnsaddr/boot2.bitping.com`), which is what every
    /// stock install should use. Override for staging environments or
    /// self-hosted hub testing — e.g. `/ip4/10.0.0.5/tcp/45445` or
    /// `/dnsaddr/boot-staging.example.com`.
    #[serde(default = "default_bootstrap")]
    pub bootstrap: Multiaddr,
}

fn default_bootstrap() -> Multiaddr {
    "/dnsaddr/boot2.bitping.com"
        .parse()
        .expect("hardcoded bootstrap multiaddr must parse")
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
    /// Country filter. Stored as Alpha-2 (the wire format the hub's
    /// `FindNodes` requirements expect). Accepts Alpha-2 ("RU"), Alpha-3
    /// ("RUS"), or country name ("Russia" / "Russian Federation") on
    /// input — all normalised to Alpha-2 via `keshvar` during deserialise.
    #[serde(default, deserialize_with = "deserialize_country_alpha2")]
    pub country: Option<String>,
    #[serde(default = "default_min_bandwith")]
    #[serde(with = "human_bandwidth::serde")]
    pub min_bandwidth: Bandwidth,
}

/// Accepts Alpha-2, Alpha-3, or a country name (case-insensitive) and
/// normalises to the Alpha-2 string the hub expects. Returns a serde error
/// with the rejected input if no match exists, so a typo in `Config.yaml`
/// fails loudly at startup rather than silently routing to "no country
/// filter" and proxying via arbitrary nodes.
fn deserialize_country_alpha2<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw = Option::<String>::deserialize(deserializer)?;
    let Some(raw) = raw else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_country_alpha2(trimmed)
        .map(Some)
        .ok_or_else(|| D::Error::custom(format!("unknown country: {trimmed:?}")))
}

/// Public so tests / callers can validate without going through serde.
pub fn parse_country_alpha2(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let upper = trimmed.to_ascii_uppercase();

    // Alpha-2 fast path: keshvar::Alpha2 is an enum whose variants are the
    // ISO codes, so its `TryFrom<&str>` is the cheapest exact match.
    if upper.len() == 2 {
        if let Ok(a2) = keshvar::Alpha2::try_from(upper.as_str()) {
            return Some(a2.to_string());
        }
    }

    // Alpha-3 → look up the Country and pull its alpha2 back out.
    if upper.len() == 3 {
        if let Ok(a3) = keshvar::Alpha3::try_from(upper.as_str()) {
            return Some(a3.to_country().alpha2().to_string());
        }
    }

    // Fall back to keshvar's ISO short-name + long-name lookup. Both lazily
    // build a HashMap behind `#[cfg(feature = "search-…")]`. We deliberately
    // skip `search-translations`/`find_by_name` — that variant loads every
    // localised alias for every country and was observed to blow the test
    // stack on first access. Short + long ISO names already cover
    // "Russia"/"Russian Federation", "South Korea"/"Republic of Korea", etc.
    let needle = trimmed.to_ascii_lowercase();
    if let Ok(c) = keshvar::find_by_iso_short_name(&needle) {
        return Some(c.alpha2().to_string());
    }
    if let Ok(c) = keshvar::find_by_iso_long_name(&needle) {
        return Some(c.alpha2().to_string());
    }

    None
}

#[cfg(test)]
mod country_parse_tests {
    use super::parse_country_alpha2;

    #[test]
    fn alpha2_passthrough() {
        assert_eq!(parse_country_alpha2("RU").as_deref(), Some("RU"));
        assert_eq!(parse_country_alpha2("ru").as_deref(), Some("RU"));
    }

    #[test]
    fn alpha3_converts() {
        assert_eq!(parse_country_alpha2("RUS").as_deref(), Some("RU"));
    }

    #[test]
    fn iso_short_name() {
        assert_eq!(parse_country_alpha2("Russian Federation").as_deref(), Some("RU"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_country_alpha2("Atlantis").is_none());
    }
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
    /// Load config from `Config.yaml` in the current working directory and
    /// override with environment variables. Same behaviour as before — kept
    /// for callers that don't care about the path.
    pub fn new() -> eyre::Result<Config> {
        Self::from_path("Config.yaml")
    }

    /// Load config from an explicit YAML path. Used by the `--config <path>`
    /// CLI flag so customers can keep `Config.yaml` anywhere on disk
    /// (e.g. `~/.config/p2proxy/Config.yaml`, `/etc/p2proxy/Config.yaml`).
    /// Environment variable overrides still apply.
    pub fn from_path<P: AsRef<std::path::Path>>(path: P) -> eyre::Result<Config> {
        Ok(Figment::new()
            .merge(Yaml::file(path.as_ref()))
            .merge(Env::raw())
            .extract()?)
    }
}
