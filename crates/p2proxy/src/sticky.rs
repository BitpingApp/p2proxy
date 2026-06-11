//! Sticky exit-peer persistence (BIT-597): remember which peer each
//! discovery-driven server selected so restarts and reconnects re-use the
//! same exit (same egress IP, as stable as that node's IP) instead of
//! rotating to an arbitrary new match. Best-effort — a remembered peer that's
//! gone just falls back to discovery and the replacement is remembered.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use libp2p::PeerId;
use metrics::counter;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

const STORE_VERSION: u32 = 1;

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
struct StickyEntry {
    peer_id: PeerId,
    /// `ServerPeerOptions::filter_fingerprint` at the time the peer was
    /// chosen. A mismatch on load means the server's filters changed —
    /// the remembered peer may no longer match them, so it's dropped.
    fingerprint: String,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StickyFile {
    version: u32,
    servers: HashMap<u16, StickyEntry>,
}

/// Owned exclusively by the swarm task (a plain field on `Bootstrapped`) —
/// no locking; every mutation saves the whole file atomically.
#[derive(Debug)]
pub struct StickyStore {
    path: PathBuf,
    entries: HashMap<u16, StickyEntry>,
}

impl StickyStore {
    /// Load from `path`. Missing, corrupt, or wrong-version files yield an
    /// empty store — sticky state is a cache, never worth failing startup
    /// over. The next `remember` rewrites a valid file.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = match std::fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                warn!(?path, ?e, "could not read sticky store — starting empty");
                HashMap::new()
            }
            Ok(raw) => match serde_json::from_str::<StickyFile>(&raw) {
                Ok(file) if file.version == STORE_VERSION => file.servers,
                Ok(file) => {
                    warn!(
                        ?path,
                        version = file.version,
                        "sticky store has unknown version — starting empty"
                    );
                    HashMap::new()
                }
                Err(e) => {
                    warn!(?path, ?e, "sticky store is corrupt — starting empty");
                    HashMap::new()
                }
            },
        };
        Self { path, entries }
    }

    /// The remembered exit peer for `port`, provided the server's filters
    /// haven't changed since it was chosen. A fingerprint mismatch drops the
    /// entry — silently reusing a peer that may no longer match the
    /// configured country/bandwidth would defeat the filters.
    pub fn get(&mut self, port: u16, fingerprint: &str) -> Option<PeerId> {
        let entry = self.entries.get(&port)?;
        if entry.fingerprint == fingerprint {
            return Some(entry.peer_id);
        }
        counter!("p2proxy_sticky_invalidated_total", "port" => port.to_string()).increment(1);
        debug!(
            port,
            old = entry.fingerprint,
            new = fingerprint,
            "sticky entry invalidated — server filters changed"
        );
        self.entries.remove(&port);
        self.save_best_effort();
        None
    }

    /// Record `peer` as the exit for `port`. Returns `true` when this
    /// changed the remembered peer (callers use it to log the change once).
    pub fn remember(
        &mut self,
        port: u16,
        fingerprint: &str,
        peer: PeerId,
    ) -> Result<bool, StickyStoreError> {
        if self
            .entries
            .get(&port)
            .is_some_and(|e| e.peer_id == peer && e.fingerprint == fingerprint)
        {
            return Ok(false);
        }
        self.entries.insert(
            port,
            StickyEntry {
                peer_id: peer,
                fingerprint: fingerprint.to_string(),
                updated_at: Utc::now(),
            },
        );
        self.save()?;
        Ok(true)
    }

    /// Drop the remembered peer for `port` (it failed to connect).
    pub fn forget(&mut self, port: u16) {
        if self.entries.remove(&port).is_some() {
            self.save_best_effort();
        }
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
        assert_eq!(store.get(1080, "v1|1080||50000000"), None);
    }

    #[test]
    fn remember_roundtrips_across_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let peer = random_peer();
        let fp = "v1|1080|NL|50000000";

        let mut store = store_in(&dir);
        assert!(store.remember(1080, fp, peer).expect("save"));

        let mut reloaded = store_in(&dir);
        assert_eq!(reloaded.get(1080, fp), Some(peer));
    }

    #[test]
    fn fingerprint_mismatch_invalidates_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let peer = random_peer();

        let mut store = store_in(&dir);
        store.remember(1080, "v1|1080|NL|50000000", peer).expect("save");
        assert_eq!(store.get(1080, "v1|1080|RU|50000000"), None);

        let mut reloaded = store_in(&dir);
        assert_eq!(
            reloaded.get(1080, "v1|1080|NL|50000000"),
            None,
            "invalidation persists to disk"
        );
    }

    #[test]
    fn corrupt_file_loads_empty_and_next_save_recovers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sticky_peers.json");
        std::fs::write(&path, "{not json").expect("write garbage");

        let mut store = StickyStore::load(&path);
        let peer = random_peer();
        let fp = "v1|1080||50000000";
        assert_eq!(store.get(1080, fp), None);
        store.remember(1080, fp, peer).expect("save over corrupt file");

        let mut reloaded = StickyStore::load(&path);
        assert_eq!(reloaded.get(1080, fp), Some(peer));
    }

    #[test]
    fn unchanged_remember_reports_no_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let peer = random_peer();
        let fp = "v1|1080||50000000";

        let mut store = store_in(&dir);
        assert!(store.remember(1080, fp, peer).expect("save"));
        assert!(!store.remember(1080, fp, peer).expect("no-op save"));
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = store_in(&dir);
        store
            .remember(1080, "v1|1080||50000000", random_peer())
            .expect("save");
        assert!(!dir.path().join("sticky_peers.json.tmp").exists());
        assert!(dir.path().join("sticky_peers.json").exists());
    }

    #[test]
    fn forget_drops_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let peer = random_peer();
        let fp = "v1|1080||50000000";

        let mut store = store_in(&dir);
        store.remember(1080, fp, peer).expect("save");
        store.forget(1080);
        assert_eq!(store.get(1080, fp), None);

        let mut reloaded = store_in(&dir);
        assert_eq!(reloaded.get(1080, fp), None);
    }
}
