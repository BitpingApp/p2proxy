use std::{
    borrow::Cow,
    fmt::Display,
    str::FromStr,
};

use color_eyre::eyre;
use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use human_bandwidth::re::bandwidth::Bandwidth;
use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    /// Ordered pinned-peer preference list (BIT-597). Each entry is a bare
    /// peer id (`12D3Koo…`, preferred — the route is resolved through the hub
    /// at connect time) or a full multiaddr ending in `/p2p/<peer-id>`
    /// (dialed verbatim in addition to any hub-resolved route). The first
    /// entry is always tried first; later entries are failovers.
    #[serde(default)]
    pub destination_peers: Option<Vec<DestinationPeerEntry>>,
    /// When every pinned peer is offline/unresolvable: `false` (default)
    /// keeps retrying the list — the egress IP never silently changes;
    /// `true` falls back to country/bandwidth discovery.
    #[serde(default)]
    pub fallback_to_discovery: bool,
    /// Remember the discovered exit peer in `sticky_peers.json` and try to
    /// reuse it across restarts/reconnects for a stable egress IP. Only
    /// applies to servers without pinned peers. Default `true`.
    #[serde(default = "default_sticky")]
    pub sticky: bool,
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

fn default_sticky() -> bool {
    true
}

/// One entry of the `destination_peers` preference list: the peer's identity
/// plus, when the operator supplied a full multiaddr, the verbatim address to
/// dial alongside whatever route the hub resolves.
#[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Clone)]
pub struct DestinationPeerEntry {
    pub peer_id: PeerId,
    pub address: Option<Multiaddr>,
}

impl DestinationPeerEntry {
    /// Interpret a multiaddr entry: the LAST `/p2p/` component is the
    /// destination identity (a circuit address carries the relay's id
    /// earlier in the path). A bare `/p2p/<id>` pins identity only;
    /// anything longer is also dialable verbatim.
    pub fn from_multiaddr(addr: &Multiaddr) -> Option<Self> {
        let peer_id = addr
            .iter()
            .filter_map(|p| match p {
                Protocol::P2p(pid) => Some(pid),
                _ => None,
            })
            .last()?;
        let bare_pin = addr.iter().count() == 1;
        Some(Self {
            peer_id,
            address: (!bare_pin).then(|| addr.clone()),
        })
    }
}

impl FromStr for DestinationPeerEntry {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("destination peer entry is empty".to_string());
        }
        if !trimmed.starts_with('/') {
            let peer_id = PeerId::from_str(trimmed)
                .map_err(|e| format!("invalid peer id {trimmed:?}: {e}"))?;
            return Ok(Self {
                peer_id,
                address: None,
            });
        }
        let addr = Multiaddr::from_str(trimmed)
            .map_err(|e| format!("invalid multiaddr {trimmed:?}: {e}"))?;
        Self::from_multiaddr(&addr)
            .ok_or_else(|| format!("multiaddr {trimmed:?} must end with /p2p/<peer-id>"))
    }
}

impl Display for DestinationPeerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.address {
            Some(addr) => write!(f, "{addr}"),
            None => write!(f, "{}", self.peer_id),
        }
    }
}

impl<'de> Deserialize<'de> for DestinationPeerEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(D::Error::custom)
    }
}

impl Serialize for DestinationPeerEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Config mistakes that must fail startup loudly rather than silently route
/// through arbitrary nodes.
#[derive(Debug, Error)]
pub enum ConfigValidationError {
    #[error("server :{port}: destination_peers is present but empty — list at least one peer or remove the key")]
    EmptyPinnedList { port: u16 },
}

impl ServerPeerOptions {
    /// The ordered pinned-peer list. Empty when the server is
    /// discovery-driven.
    pub fn pinned(&self) -> Vec<DestinationPeerEntry> {
        self.destination_peers.clone().unwrap_or_default()
    }

    pub fn validate(&self, port: u16) -> Result<(), ConfigValidationError> {
        if let Some(list) = &self.destination_peers
            && list.is_empty()
        {
            return Err(ConfigValidationError::EmptyPinnedList { port });
        }
        Ok(())
    }

    /// Identity of this server's discovery filters. The sticky store records
    /// it alongside the remembered peer so a config change (different
    /// country, bandwidth floor, or port) invalidates the stale affinity
    /// instead of silently exiting through a peer that no longer matches.
    pub fn filter_fingerprint(&self, port: u16) -> String {
        format!(
            "v1|{port}|{}|{}",
            self.country.as_deref().unwrap_or(""),
            self.min_bandwidth.as_bps()
        )
    }
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
mod destination_peer_tests {
    use super::*;

    fn random_peer() -> PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
    }

    fn options(destination_peers: Option<Vec<DestinationPeerEntry>>) -> ServerPeerOptions {
        ServerPeerOptions {
            destination_peers,
            fallback_to_discovery: false,
            sticky: true,
            country: None,
            min_bandwidth: default_min_bandwith(),
        }
    }

    #[test]
    fn bare_peer_id_parses() {
        let id = random_peer();
        let entry: DestinationPeerEntry = id.to_string().parse().expect("parses");
        assert_eq!(entry.peer_id, id);
        assert_eq!(entry.address, None);
    }

    #[test]
    fn full_multiaddr_extracts_last_p2p_and_keeps_address() {
        let relay = random_peer();
        let dest = random_peer();
        let raw = format!("/dns4/hub.example.com/tcp/31515/p2p/{relay}/p2p-circuit/p2p/{dest}");
        let entry: DestinationPeerEntry = raw.parse().expect("parses");
        assert_eq!(entry.peer_id, dest, "destination is the LAST /p2p/");
        assert_eq!(entry.address, Some(raw.parse().expect("multiaddr")));
    }

    #[test]
    fn bare_p2p_multiaddr_pins_identity_only() {
        let id = random_peer();
        let entry: DestinationPeerEntry = format!("/p2p/{id}").parse().expect("parses");
        assert_eq!(entry.peer_id, id);
        assert_eq!(entry.address, None, "bare /p2p/<id> has no dialable address");
    }

    #[test]
    fn multiaddr_without_p2p_rejected() {
        let err = "/ip4/1.2.3.4/tcp/443"
            .parse::<DestinationPeerEntry>()
            .expect_err("no peer identity");
        assert!(err.contains("/p2p/"));
    }

    #[test]
    fn garbage_rejected() {
        assert!("not-a-peer".parse::<DestinationPeerEntry>().is_err());
        assert!("".parse::<DestinationPeerEntry>().is_err());
    }

    #[test]
    fn serialize_roundtrips_original_string_form() {
        let id = random_peer();
        for raw in [
            id.to_string(),
            format!("/ip4/9.9.9.9/tcp/31515/p2p/{id}"),
        ] {
            let entry: DestinationPeerEntry = raw.parse().expect("parses");
            let yaml = serde_yaml_roundtrip(&entry);
            assert_eq!(yaml.peer_id, entry.peer_id);
            assert_eq!(yaml.address, entry.address);
        }
    }

    fn serde_yaml_roundtrip(entry: &DestinationPeerEntry) -> DestinationPeerEntry {
        let json = serde_json::to_string(entry).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    #[test]
    fn validate_rejects_empty_pinned_list() {
        let opts = options(Some(vec![]));
        assert!(matches!(
            opts.validate(1080),
            Err(ConfigValidationError::EmptyPinnedList { port: 1080 })
        ));
    }

    #[test]
    fn pinned_empty_for_discovery_servers() {
        assert!(options(None).pinned().is_empty());
    }

    #[test]
    fn defaults_sticky_true_fallback_false() {
        let yaml = "country: US";
        let opts: ServerPeerOptions = serde_yaml_str(yaml);
        assert!(opts.sticky);
        assert!(!opts.fallback_to_discovery);
        assert!(opts.destination_peers.is_none());
    }

    fn serde_yaml_str(yaml: &str) -> ServerPeerOptions {
        figment::Figment::new()
            .merge(figment::providers::Yaml::string(yaml))
            .extract()
            .expect("yaml parses")
    }

    #[test]
    fn destination_peers_yaml_list_parses_in_order() {
        let (a, b) = (random_peer(), random_peer());
        let yaml = format!("destination_peers:\n  - {a}\n  - /ip4/9.9.9.9/tcp/31515/p2p/{b}\n");
        let opts = serde_yaml_str(&yaml);
        let pinned = opts.pinned();
        assert_eq!(pinned.len(), 2);
        assert_eq!(pinned[0].peer_id, a, "order preserved — rank 0 first");
        assert_eq!(pinned[1].peer_id, b);
    }

    #[test]
    fn fingerprint_changes_on_each_filter_dimension() {
        let base = options(None);
        let fp = base.filter_fingerprint(1080);
        assert_ne!(fp, base.filter_fingerprint(1081), "port changes fp");

        let mut with_country = options(None);
        with_country.country = Some("NL".to_string());
        assert_ne!(fp, with_country.filter_fingerprint(1080));

        let mut with_bw = options(None);
        with_bw.min_bandwidth = Bandwidth::from_mbps(100);
        assert_ne!(fp, with_bw.filter_fingerprint(1080));

        assert_eq!(fp, options(None).filter_fingerprint(1080), "stable");
    }
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
        let pinned = self.pinned();
        let opt_string = if let Some(first) = pinned.first() {
            match pinned.len() {
                1 => format!("Pinned: {}", first.peer_id),
                n => format!("Pinned: {} +{} more", first.peer_id, n - 1),
            }
        } else if let Some(c) = &self.country {
            format!("Country: {c}")
        } else {
            "Unknown".to_string()
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
        let config: Config = Figment::new()
            .merge(Yaml::file(path.as_ref()))
            .merge(Env::raw())
            .extract()?;
        for server in &config.servers {
            server.peer_options.validate(server.port)?;
        }
        Ok(config)
    }
}
