use libp2p::{Multiaddr, PeerId};

/// Remembered exit-peer affinity for discovery-driven servers. Production
/// persists to `sticky_peers.json`; the fake keeps it in memory. All mutations
/// are best-effort — persistence failures are logged by the adapter, never
/// surfaced here.
pub trait StickyStore {
    /// Remembered exit pool for `port` (most-recently-active first), or empty
    /// when the server's filters changed since they were chosen.
    fn pool(&mut self, port: u16, fingerprint: &str) -> Vec<PeerId>;

    /// Remembered direct address for a pool member, if one was observed.
    fn direct_address(&self, port: u16, peer: PeerId) -> Option<Multiaddr>;

    /// Promote `peer` to the front of `port`'s pool. Returns `true` when this
    /// changed which peer is at the front.
    fn remember(&mut self, port: u16, fingerprint: &str, peer: PeerId, max: usize) -> bool;

    /// Record the direct address observed for `peer` in whatever pool(s) it
    /// already belongs to. Never adds a peer — a directly-connected peer that
    /// was never adopted as an exit (e.g. a hub) must not enter the pool.
    fn note_direct_address(&mut self, peer: PeerId, address: Multiaddr);

    /// Drop one peer from `port`'s pool after it failed to reconnect.
    fn forget_peer(&mut self, port: u16, peer: PeerId);
}
