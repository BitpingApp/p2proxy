//! Sticky exit-peer persistence (BIT-597): remember which peers each
//! discovery-driven server has successfully exited through so restarts and
//! reconnects re-use the same egress (same IP, as stable as that node's IP)
//! instead of rotating to an arbitrary new match.
//!
//! Each server keeps a *pool* of proven-good exits (most-recently-active
//! first), not a single peer — so the pool auto-heals on restart: members
//! are tried in order, dead ones are pruned, and discovery refills the gap.
//! Best-effort throughout — a remembered peer that's gone just falls through
//! to the next pool member or discovery, and the replacement is remembered.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use libp2p::{Multiaddr, PeerId};
use metrics::counter;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

const STORE_VERSION: u32 = 2;

#[derive(Debug, Error)]
pub enum StickyStoreError {
    #[error("failed to serialize sticky store: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to persist sticky store to {path}: {source}")]
    Persist {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StickyPeer {
    peer_id: PeerId,
    /// Last-known direct (non-circuit) address we connected to this peer
    /// through. Tried first on reconnect to skip a hub round-trip when the
    /// peer's IP hasn't changed. Absent until a direct connection is
    /// observed (most exits are reached via relay circuit).
    #[serde(default)]
    address: Option<Multiaddr>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StickyPool {
    /// `ServerPeerOptions::filter_fingerprint` at the time these peers were
    /// chosen. A mismatch on load means the server's filters changed — the
    /// remembered peers may no longer match, so the pool is dropped.
    fingerprint: String,
    /// Proven-good exit peers, most-recently-active first.
    peers: Vec<StickyPeer>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StickyFile {
    version: u32,
    servers: HashMap<u16, StickyPool>,
}

/// Probe just the version so a v1 file can be migrated rather than discarded.
#[derive(Deserialize)]
struct VersionProbe {
    version: u32,
}

/// The v1 on-disk shape: one remembered peer per server.
#[derive(Deserialize)]
struct StickyFileV1 {
    servers: HashMap<u16, StickyEntryV1>,
}

#[derive(Deserialize)]
struct StickyEntryV1 {
    peer_id: PeerId,
    fingerprint: String,
    updated_at: DateTime<Utc>,
}

/// Owned exclusively by the swarm task (a plain field on `Bootstrapped`) —
/// no locking; every mutation saves the whole file atomically.
#[derive(Debug)]
pub struct StickyStore {
    path: PathBuf,
    entries: HashMap<u16, StickyPool>,
}

impl StickyStore {
    /// Load from `path`. Missing or corrupt files yield an empty store —
    /// sticky state is a cache, never worth failing startup over. A v1
    /// (single-peer) file is migrated to a one-member pool so existing
    /// affinity survives the upgrade. The next `remember` rewrites a valid
    /// v2 file.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = match std::fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                warn!(?path, ?e, "could not read sticky store — starting empty");
                HashMap::new()
            }
            Ok(raw) => Self::parse(&path, &raw),
        };
        Self { path, entries }
    }

    fn parse(path: &Path, raw: &str) -> HashMap<u16, StickyPool> {
        let version = serde_json::from_str::<VersionProbe>(raw)
            .map(|p| p.version)
            .unwrap_or(0);
        match version {
            STORE_VERSION => serde_json::from_str::<StickyFile>(raw)
                .map(|f| f.servers)
                .unwrap_or_else(|e| {
                    warn!(?path, ?e, "sticky store is corrupt — starting empty");
                    HashMap::new()
                }),
            1 => serde_json::from_str::<StickyFileV1>(raw)
                .map(|f| migrate_v1(f.servers))
                .unwrap_or_else(|e| {
                    warn!(?path, ?e, "v1 sticky store is corrupt — starting empty");
                    HashMap::new()
                }),
            other => {
                warn!(
                    ?path,
                    version = other,
                    "sticky store has unknown version — starting empty"
                );
                HashMap::new()
            }
        }
    }

    /// The remembered exit pool for `port` (most-recently-active first),
    /// provided the server's filters haven't changed since they were
    /// chosen. A fingerprint mismatch drops the whole pool — silently
    /// reusing peers that may no longer match the configured
    /// country/bandwidth would defeat the filters.
    pub fn pool(&mut self, port: u16, fingerprint: &str) -> Vec<PeerId> {
        let Some(pool) = self.entries.get(&port) else {
            return Vec::new();
        };
        if pool.fingerprint == fingerprint {
            return pool.peers.iter().map(|p| p.peer_id).collect();
        }
        counter!("p2proxy_sticky_invalidated_total", "port" => port.to_string()).increment(1);
        debug!(
            port,
            old = pool.fingerprint,
            new = fingerprint,
            "sticky pool invalidated — server filters changed"
        );
        self.entries.remove(&port);
        self.save_best_effort();
        Vec::new()
    }

    /// The remembered direct address for a pool member, if one was observed.
    /// Tried first on reconnect so a peer whose IP is unchanged comes back
    /// without a hub round-trip.
    pub fn direct_address(&self, port: u16, peer: PeerId) -> Option<Multiaddr> {
        self.entries
            .get(&port)?
            .peers
            .iter()
            .find(|p| p.peer_id == peer)
            .and_then(|p| p.address.clone())
    }

    /// Promote `peer` to the front of `port`'s pool (most-recently-active),
    /// capping at `max`. A fingerprint change resets the pool. Returns
    /// `true` when this changed which peer is at the front (callers log the
    /// change once). Preserves an existing member's remembered address.
    pub fn remember(
        &mut self,
        port: u16,
        fingerprint: &str,
        peer: PeerId,
        max: usize,
    ) -> Result<bool, StickyStoreError> {
        let max = max.max(1);
        let pool = self.entries.entry(port).or_insert_with(|| StickyPool {
            fingerprint: fingerprint.to_string(),
            peers: Vec::new(),
        });
        if pool.fingerprint != fingerprint {
            pool.fingerprint = fingerprint.to_string();
            pool.peers.clear();
        }

        let was_front = pool.peers.first().map(|p| p.peer_id) == Some(peer);
        let mut entry = pool
            .peers
            .iter()
            .position(|p| p.peer_id == peer)
            .map(|i| pool.peers.remove(i))
            .unwrap_or(StickyPeer {
                peer_id: peer,
                address: None,
                updated_at: Utc::now(),
            });
        entry.updated_at = Utc::now();
        pool.peers.insert(0, entry);
        pool.peers.truncate(max);
        self.save()?;
        Ok(!was_front)
    }

    /// Promote a directly-connected `peer` into `port`'s pool with its real
    /// `address`. This is how the pool fills with *proven, directly-reachable*
    /// exits rather than the relay-only candidates the hub hands out: when the
    /// pool is full, a relay-only (address-less) standby is evicted to make
    /// room — never the front (active) peer. No-op if the pool is already full
    /// of address-having members. Returns `true` when the pool changed.
    pub fn promote_connected(
        &mut self,
        port: u16,
        fingerprint: &str,
        peer: PeerId,
        address: Multiaddr,
        max: usize,
    ) -> bool {
        let max = max.max(1);
        let pool = self.entries.entry(port).or_insert_with(|| StickyPool {
            fingerprint: fingerprint.to_string(),
            peers: Vec::new(),
        });
        if pool.fingerprint != fingerprint {
            pool.fingerprint = fingerprint.to_string();
            pool.peers.clear();
        }

        // Already pooled — just record/refresh its address.
        if let Some(entry) = pool.peers.iter_mut().find(|p| p.peer_id == peer) {
            if entry.address.as_ref() == Some(&address) {
                return false;
            }
            entry.address = Some(address);
            self.save_best_effort();
            return true;
        }

        if pool.peers.len() >= max {
            // Make room by dropping a relay-only standby — never the front
            // (active) peer. If every standby already has a direct address,
            // leave the full pool as-is.
            let Some(idx) = pool
                .peers
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, p)| p.address.is_none())
                .map(|(i, _)| i)
            else {
                return false;
            };
            pool.peers.remove(idx);
        }
        pool.peers.push(StickyPeer {
            peer_id: peer,
            address: Some(address),
            updated_at: Utc::now(),
        });
        self.save_best_effort();
        true
    }

    /// Record the direct address we connected to `peer` through — in
    /// whichever server pool(s) it belongs to — so a future reconnect can try
    /// it before asking the hub (and the NETWORK tab shows it). The hub only
    /// hands out relay routes for NAT'd peers, so a member's real egress IP is
    /// only ever learned here, from a live (DCUtR-upgraded) connection. No-op
    /// if the peer isn't pooled anywhere or the address is unchanged.
    pub fn note_direct_address(&mut self, peer: PeerId, address: Multiaddr) {
        let mut changed = false;
        for pool in self.entries.values_mut() {
            let Some(entry) = pool.peers.iter_mut().find(|p| p.peer_id == peer) else {
                continue;
            };
            if entry.address.as_ref() != Some(&address) {
                entry.address = Some(address.clone());
                changed = true;
            }
        }
        if changed {
            self.save_best_effort();
        }
    }

    /// Drop one peer from `port`'s pool — it failed to reconnect after
    /// retries, so it shouldn't be tried again until rediscovered.
    pub fn forget_peer(&mut self, port: u16, peer: PeerId) {
        let removed = self
            .entries
            .get_mut(&port)
            .map(|pool| {
                let before = pool.peers.len();
                pool.peers.retain(|p| p.peer_id != peer);
                before != pool.peers.len()
            })
            .unwrap_or(false);
        if !removed {
            return;
        }
        if self.entries.get(&port).is_some_and(|p| p.peers.is_empty()) {
            self.entries.remove(&port);
        }
        self.save_best_effort();
    }

    fn save(&self) -> Result<(), StickyStoreError> {
        let file = StickyFile {
            version: STORE_VERSION,
            servers: self.entries.clone(),
        };
        let json = serde_json::to_string_pretty(&file)?;
        // Write-then-rename so a crash mid-write never leaves a truncated
        // store (load treats corrupt files as empty, losing all affinity).
        let tmp = self.path.with_extension("json.tmp");
        let persist = || -> std::io::Result<()> {
            std::fs::write(&tmp, &json)?;
            std::fs::rename(&tmp, &self.path)
        };
        persist().map_err(|source| StickyStoreError::Persist {
            path: self.path.clone(),
            source,
        })
    }

    fn save_best_effort(&self) {
        if let Err(e) = self.save() {
            warn!(?e, "failed to persist sticky store");
        }
    }
}

/// Turn a v1 single-peer-per-server file into v2 one-member pools so an
/// existing remembered exit isn't lost on upgrade.
fn migrate_v1(servers: HashMap<u16, StickyEntryV1>) -> HashMap<u16, StickyPool> {
    servers
        .into_iter()
        .map(|(port, e)| {
            (
                port,
                StickyPool {
                    fingerprint: e.fingerprint,
                    peers: vec![StickyPeer {
                        peer_id: e.peer_id,
                        address: None,
                        updated_at: e.updated_at,
                    }],
                },
            )
        })
        .collect()
}

/// Default store location — CWD, next to `node_keypair.bin` (same
/// persistence convention as the client identity).
pub fn default_sticky_path() -> &'static Path {
    Path::new("sticky_peers.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_peer() -> PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
    }

    fn store_in(dir: &tempfile::TempDir) -> StickyStore {
        StickyStore::load(dir.path().join("sticky_peers.json"))
    }

    #[test]
    fn missing_file_loads_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = store_in(&dir);
        assert!(store.pool(1080, "v1|1080||50000000").is_empty());
    }

    #[test]
    fn remember_roundtrips_across_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let peer = random_peer();
        let fp = "v1|1080|NL|50000000";

        let mut store = store_in(&dir);
        assert!(store.remember(1080, fp, peer, 5).expect("save"));

        let mut reloaded = store_in(&dir);
        assert_eq!(reloaded.pool(1080, fp), vec![peer]);
    }

    #[test]
    fn pool_keeps_most_recently_active_first_and_caps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fp = "v1|1080||1000";
        let peers: Vec<PeerId> = (0..4).map(|_| random_peer()).collect();

        let mut store = store_in(&dir);
        for p in &peers {
            store.remember(1080, fp, *p, 3).expect("save");
        }

        // Cap 3, MRU-first: the last three remembered, newest first.
        let mut reloaded = store_in(&dir);
        assert_eq!(reloaded.pool(1080, fp), vec![peers[3], peers[2], peers[1]]);
    }

    #[test]
    fn re_remembering_promotes_without_duplicating() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fp = "v1|1080||1000";
        let a = random_peer();
        let b = random_peer();

        let mut store = store_in(&dir);
        store.remember(1080, fp, a, 5).expect("save");
        store.remember(1080, fp, b, 5).expect("save");
        // Re-adopt a — it moves to the front, no duplicate.
        assert!(store.remember(1080, fp, a, 5).expect("save"));
        assert_eq!(store.pool(1080, fp), vec![a, b]);
        // Re-remembering the front peer reports no change.
        assert!(!store.remember(1080, fp, a, 5).expect("save"));
    }

    #[test]
    fn fingerprint_mismatch_drops_whole_pool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = store_in(&dir);
        store
            .remember(1080, "v1|1080|NL|50000000", random_peer(), 5)
            .expect("save");
        assert!(store.pool(1080, "v1|1080|RU|50000000").is_empty());

        let mut reloaded = store_in(&dir);
        assert!(
            reloaded.pool(1080, "v1|1080|NL|50000000").is_empty(),
            "invalidation persists to disk"
        );
    }

    #[test]
    fn direct_address_roundtrips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fp = "v1|1080||1000";
        let peer = random_peer();
        let addr: Multiaddr = "/ip4/203.0.113.7/udp/45445/quic-v1".parse().expect("addr");

        let mut store = store_in(&dir);
        store.remember(1080, fp, peer, 5).expect("save");
        // No port needed — the address is recorded wherever the peer is pooled.
        store.note_direct_address(peer, addr.clone());

        let mut reloaded = store_in(&dir);
        // pool() validates the fingerprint and keeps the entry.
        assert_eq!(reloaded.pool(1080, fp), vec![peer]);
        assert_eq!(reloaded.direct_address(1080, peer), Some(addr));
    }

    #[test]
    fn promote_connected_fills_with_addresses_and_evicts_relay_only_standbys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fp = "v1|1080||1000";
        let active = random_peer();
        let a = random_peer();
        let b = random_peer();
        let addr = |p: PeerId| -> Multiaddr {
            format!("/ip4/198.51.100.1/tcp/443/p2p/{p}")
                .parse()
                .expect("addr")
        };

        let mut store = store_in(&dir);
        // Active at the front, then a relay-only standby (no address).
        store.remember(1080, fp, active, 3).expect("save");
        store.remember(1080, fp, a, 3).expect("save");
        // Oops — remember promotes to front; reset so `active` is front again.
        store.remember(1080, fp, active, 3).expect("save");

        // A directly-connected peer joins at the back with its address.
        assert!(store.promote_connected(1080, fp, b, addr(b), 3));
        assert_eq!(store.pool(1080, fp), vec![active, a, b]);
        assert_eq!(store.direct_address(1080, b), Some(addr(b)));

        // Re-promoting just refreshes the address (no duplicate).
        assert!(!store.promote_connected(1080, fp, b, addr(b), 3));
        assert_eq!(store.pool(1080, fp), vec![active, a, b]);

        // Pool is full (active, b w/addr, a relay-only). A new connected peer
        // evicts the relay-only standby `a`, never the front `active`.
        let c = random_peer();
        assert!(store.promote_connected(1080, fp, c, addr(c), 3));
        let pool = store.pool(1080, fp);
        assert_eq!(pool[0], active, "front (active) is never evicted");
        assert!(pool.contains(&c) && pool.contains(&b));
        assert!(!pool.contains(&a), "relay-only standby was evicted");

        let mut reloaded = store_in(&dir);
        assert_eq!(reloaded.direct_address(1080, c), Some(addr(c)));
    }

    #[test]
    fn forget_peer_removes_one_keeps_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fp = "v1|1080||1000";
        let a = random_peer();
        let b = random_peer();

        let mut store = store_in(&dir);
        store.remember(1080, fp, a, 5).expect("save");
        store.remember(1080, fp, b, 5).expect("save");
        store.forget_peer(1080, b);
        assert_eq!(store.pool(1080, fp), vec![a]);

        let mut reloaded = store_in(&dir);
        assert_eq!(reloaded.pool(1080, fp), vec![a]);
    }

    #[test]
    fn v1_file_migrates_to_one_member_pool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticky_peers.json");
        let peer = random_peer();
        let v1 = format!(
            r#"{{"version":1,"servers":{{"1080":{{"peer_id":"{peer}","fingerprint":"v1|1080||1000","updated_at":"2026-06-17T11:41:44.951690Z"}}}}}}"#
        );
        std::fs::write(&path, v1).expect("write v1");

        let mut store = StickyStore::load(&path);
        assert_eq!(store.pool(1080, "v1|1080||1000"), vec![peer]);

        // Next save rewrites as v2 and the migrated peer survives a reload.
        store
            .remember(1080, "v1|1080||1000", peer, 5)
            .expect("save");
        let mut reloaded = StickyStore::load(&path);
        assert_eq!(reloaded.pool(1080, "v1|1080||1000"), vec![peer]);
    }

    #[test]
    fn corrupt_file_loads_empty_and_next_save_recovers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticky_peers.json");
        std::fs::write(&path, "{not json").expect("write garbage");

        let mut store = StickyStore::load(&path);
        let peer = random_peer();
        let fp = "v1|1080||50000000";
        assert!(store.pool(1080, fp).is_empty());
        store
            .remember(1080, fp, peer, 5)
            .expect("save over corrupt file");

        let mut reloaded = StickyStore::load(&path);
        assert_eq!(reloaded.pool(1080, fp), vec![peer]);
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = store_in(&dir);
        store
            .remember(1080, "v1|1080||50000000", random_peer(), 5)
            .expect("save");
        assert!(!dir.path().join("sticky_peers.json.tmp").exists());
        assert!(dir.path().join("sticky_peers.json").exists());
    }
}
