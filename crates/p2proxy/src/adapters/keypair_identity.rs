use std::path::Path;
use std::sync::Arc;

use libp2p::PeerId;
use libp2p::identity::Keypair;
use proxy_core::errors::AuthError;
use proxy_core::ports::Identity;
use tracing::warn;

/// The node's on-disk libp2p identity.
pub struct KeypairIdentity {
    keypair: Arc<Keypair>,
}

impl KeypairIdentity {
    pub fn new(keypair: Arc<Keypair>) -> Self {
        Self { keypair }
    }
}

impl Identity for KeypairIdentity {
    fn peer_id(&self) -> PeerId {
        self.keypair.public().to_peer_id()
    }

    fn public_b58(&self) -> Result<String, AuthError> {
        let ed = self
            .keypair
            .public()
            .try_into_ed25519()
            .map_err(|e| AuthError::Signing(e.to_string()))?;
        base58_monero::encode_check(&ed.to_bytes()).map_err(|e| AuthError::Signing(e.to_string()))
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, AuthError> {
        self.keypair
            .sign(message)
            .map_err(|e| AuthError::Signing(e.to_string()))
    }
}

/// Load the persisted node identity, generating and saving a fresh Ed25519
/// keypair when the file is missing or unreadable.
pub fn load_or_generate_keypair(path: &Path) -> Keypair {
    if path.exists() {
        match std::fs::read(path) {
            Ok(bytes) => match Keypair::from_protobuf_encoding(&bytes) {
                Ok(keypair) => return keypair,
                Err(e) => warn!(?e, "could not deserialize keypair — generating a new one"),
            },
            Err(e) => warn!(?e, "could not read keypair file — generating a new one"),
        }
    }

    let keypair = Keypair::generate_ed25519();
    match keypair.to_protobuf_encoding() {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(path, &bytes) {
                warn!(?e, "failed to persist keypair");
            }
        }
        Err(e) => warn!(?e, "failed to serialize keypair"),
    }
    keypair
}
