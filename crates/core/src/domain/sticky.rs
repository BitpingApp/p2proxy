use std::collections::HashMap;

use chrono::{DateTime, Utc};
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

use crate::ports::StickyStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickyPeer {
    pub peer_id: PeerId,
    #[serde(default)]
    pub address: Option<Multiaddr>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickyPool {
    pub fingerprint: String,
    pub peers: Vec<StickyPeer>,
}

/// In-memory exit-peer affinity: per listen port, a pool of proven-good exits
/// (most-recently-active first). Pure — the file adapter wraps this with load /
/// atomic-save; tests use it directly so `connect` exercises the real logic.
#[derive(Debug, Clone, Default)]
pub struct StickyState {
    pools: HashMap<u16, StickyPool>,
}

impl StickyState {
    pub fn from_pools(pools: HashMap<u16, StickyPool>) -> Self {
        Self { pools }
    }

    pub fn pools(&self) -> &HashMap<u16, StickyPool> {
        &self.pools
    }
}

impl StickyStore for StickyState {
    fn pool(&mut self, port: u16, fingerprint: &str) -> Vec<PeerId> {
        let Some(pool) = self.pools.get(&port) else {
            return Vec::new();
        };
        if pool.fingerprint == fingerprint {
            return pool.peers.iter().map(|p| p.peer_id).collect();
        }
        self.pools.remove(&port);
        Vec::new()
    }

    fn direct_address(&self, port: u16, peer: PeerId) -> Option<Multiaddr> {
        self.pools
            .get(&port)?
            .peers
            .iter()
            .find(|p| p.peer_id == peer)
            .and_then(|p| p.address.clone())
    }

    fn remember(&mut self, port: u16, fingerprint: &str, peer: PeerId, max: usize) -> bool {
        let max = max.max(1);
        let pool = self.pools.entry(port).or_insert_with(|| StickyPool {
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
        !was_front
    }

    fn note_direct_address(&mut self, peer: PeerId, address: Multiaddr) {
        for pool in self.pools.values_mut() {
            if let Some(entry) = pool.peers.iter_mut().find(|p| p.peer_id == peer)
                && entry.address.as_ref() != Some(&address)
            {
                entry.address = Some(address.clone());
            }
        }
    }

    fn forget_peer(&mut self, port: u16, peer: PeerId) {
        let Some(pool) = self.pools.get_mut(&port) else {
            return;
        };
        pool.peers.retain(|p| p.peer_id != peer);
        if pool.peers.is_empty() {
            self.pools.remove(&port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_peer() -> PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
    }

    fn addr(p: PeerId) -> Multiaddr {
        format!("/ip4/198.51.100.1/tcp/443/p2p/{p}")
            .parse()
            .expect("addr")
    }

    #[test]
    fn pool_is_empty_when_unknown() {
        let mut s = StickyState::default();
        assert!(s.pool(1080, "fp").is_empty());
    }

    #[test]
    fn remember_keeps_mru_first_and_caps() {
        let mut s = StickyState::default();
        let peers: Vec<PeerId> = (0..4).map(|_| random_peer()).collect();
        for p in &peers {
            s.remember(1080, "fp", *p, 3);
        }
        assert_eq!(s.pool(1080, "fp"), vec![peers[3], peers[2], peers[1]]);
    }

    #[test]
    fn re_remembering_promotes_without_duplicating() {
        let mut s = StickyState::default();
        let (a, b) = (random_peer(), random_peer());
        s.remember(1080, "fp", a, 5);
        s.remember(1080, "fp", b, 5);
        assert!(s.remember(1080, "fp", a, 5));
        assert_eq!(s.pool(1080, "fp"), vec![a, b]);
        assert!(!s.remember(1080, "fp", a, 5));
    }

    #[test]
    fn fingerprint_mismatch_drops_whole_pool() {
        let mut s = StickyState::default();
        s.remember(1080, "fp-NL", random_peer(), 5);
        assert!(s.pool(1080, "fp-RU").is_empty());
        assert!(s.pool(1080, "fp-NL").is_empty(), "invalidation is sticky");
    }

    #[test]
    fn forget_removes_one_keeps_rest() {
        let mut s = StickyState::default();
        let (a, b) = (random_peer(), random_peer());
        s.remember(1080, "fp", a, 5);
        s.remember(1080, "fp", b, 5);
        s.forget_peer(1080, b);
        assert_eq!(s.pool(1080, "fp"), vec![a]);
    }

    #[test]
    fn note_direct_address_updates_existing_but_never_adds() {
        let mut s = StickyState::default();
        let p = random_peer();
        let hub = random_peer();
        s.remember(1080, "fp", p, 5);

        s.note_direct_address(p, addr(p));
        assert_eq!(s.direct_address(1080, p), Some(addr(p)));

        // A peer we merely connected to (e.g. a hub) is never added to the pool.
        s.note_direct_address(hub, addr(hub));
        assert!(!s.pool(1080, "fp").contains(&hub), "non-exit never added");
    }
}
