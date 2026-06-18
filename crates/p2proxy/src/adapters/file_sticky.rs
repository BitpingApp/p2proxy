use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use libp2p::{Multiaddr, PeerId};
use proxy_core::domain::sticky::{StickyPeer, StickyPool, StickyState};
use proxy_core::errors::StickyStoreError;
use proxy_core::events::PoolPeer;
use proxy_core::ports::StickyStore;
use serde::{Deserialize, Serialize};
use tracing::warn;

const STORE_VERSION: u32 = 2;

/// File-backed `StickyStore`: the pure pool logic lives in
/// `proxy_core::domain::sticky::StickyState`; this wraps it with versioned,
/// atomically-written JSON persistence next to `node_keypair.bin`.
pub struct FileStickyStore {
    path: PathBuf,
    state: StickyState,
}

#[derive(Serialize, Deserialize)]
struct StickyFile {
    version: u32,
    servers: HashMap<u16, StickyPool>,
}

#[derive(Deserialize)]
struct VersionProbe {
    version: u32,
}

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

impl FileStickyStore {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let pools = match std::fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                warn!(?path, ?e, "could not read sticky store — starting empty");
                HashMap::new()
            }
            Ok(raw) => parse(&path, &raw),
        };
        Self {
            path,
            state: StickyState::from_pools(pools),
        }
    }

    fn save(&self) -> Result<(), StickyStoreError> {
        let file = StickyFile {
            version: STORE_VERSION,
            servers: self.state.pools().clone(),
        };
        let json = serde_json::to_string_pretty(&file)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .and_then(|_| std::fs::rename(&tmp, &self.path))
            .map_err(|source| StickyStoreError::Persist {
                path: self.path.clone(),
                source,
            })
    }

    fn save_best_effort(&self) {
        if let Err(e) = self.save() {
            warn!(?e, "failed to persist sticky store");
        }
    }

    /// Snapshot the remembered pool for the NETWORK tab: every standby peer
    /// with its stored direct address (if any). Read-only — unlike `pool()` it
    /// never invalidates the pool on a fingerprint mismatch.
    pub fn snapshot(&self, port: u16, fingerprint: &str) -> Vec<PoolPeer> {
        self.state
            .pools()
            .get(&port)
            .filter(|pool| pool.fingerprint == fingerprint)
            .map(|pool| {
                pool.peers
                    .iter()
                    .map(|p| PoolPeer {
                        peer_id: p.peer_id,
                        addresses: p.address.clone().into_iter().collect(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl StickyStore for FileStickyStore {
    fn pool(&mut self, port: u16, fingerprint: &str) -> Vec<PeerId> {
        let pool = self.state.pool(port, fingerprint);
        self.save_best_effort();
        pool
    }

    fn direct_address(&self, port: u16, peer: PeerId) -> Option<Multiaddr> {
        self.state.direct_address(port, peer)
    }

    fn remember(&mut self, port: u16, fingerprint: &str, peer: PeerId, max: usize) -> bool {
        let changed = self.state.remember(port, fingerprint, peer, max);
        self.save_best_effort();
        changed
    }

    fn note_direct_address(&mut self, peer: PeerId, address: Multiaddr) {
        self.state.note_direct_address(peer, address);
        self.save_best_effort();
    }

    fn forget_peer(&mut self, port: u16, peer: PeerId) {
        self.state.forget_peer(port, peer);
        self.save_best_effort();
    }
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
            warn!(?path, version = other, "unknown sticky store version — starting empty");
            HashMap::new()
        }
    }
}

fn migrate_v1(servers: HashMap<u16, StickyEntryV1>) -> HashMap<u16, StickyPool> {
    servers
        .into_iter()
        .map(|(port, entry)| {
            (
                port,
                StickyPool {
                    fingerprint: entry.fingerprint,
                    peers: vec![StickyPeer {
                        peer_id: entry.peer_id,
                        address: None,
                        updated_at: entry.updated_at,
                    }],
                },
            )
        })
        .collect()
}

/// Default store location — CWD, next to `node_keypair.bin`.
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

    #[test]
    fn remember_roundtrips_across_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticky_peers.json");
        let peer = random_peer();

        let mut store = FileStickyStore::load(&path);
        store.remember(1080, "fp", peer, 5);

        let mut reloaded = FileStickyStore::load(&path);
        assert_eq!(reloaded.pool(1080, "fp"), vec![peer]);
    }

    #[test]
    fn v1_file_migrates_to_one_member_pool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticky_peers.json");
        let peer = random_peer();
        let v1 = format!(
            r#"{{"version":1,"servers":{{"1080":{{"peer_id":"{peer}","fingerprint":"fp","updated_at":"2026-06-17T11:41:44.951690Z"}}}}}}"#
        );
        std::fs::write(&path, v1).expect("write v1");

        let mut store = FileStickyStore::load(&path);
        assert_eq!(store.pool(1080, "fp"), vec![peer]);
    }

    #[test]
    fn corrupt_file_loads_empty_and_next_save_recovers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticky_peers.json");
        std::fs::write(&path, "{not json").expect("write garbage");

        let mut store = FileStickyStore::load(&path);
        let peer = random_peer();
        assert!(store.pool(1080, "fp").is_empty());
        store.remember(1080, "fp", peer, 5);

        let mut reloaded = FileStickyStore::load(&path);
        assert_eq!(reloaded.pool(1080, "fp"), vec![peer]);
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticky_peers.json");
        let mut store = FileStickyStore::load(&path);
        store.remember(1080, "fp", random_peer(), 5);
        assert!(!dir.path().join("sticky_peers.json.tmp").exists());
        assert!(path.exists());
    }
}
