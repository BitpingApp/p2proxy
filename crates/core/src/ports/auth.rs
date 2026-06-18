use std::future::Future;

use libp2p::PeerId;
use thiserror::Error;

/// The node's libp2p identity: its peer id, base58 public key, and the ability
/// to sign hub requests. Production wraps the on-disk Ed25519 keypair.
pub trait Identity {
    fn peer_id(&self) -> PeerId;
    fn public_b58(&self) -> Result<String, AuthError>;
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, AuthError>;
}

/// Exchanges the configured Bitping API key for a federated PASETO. Production
/// calls the gRPC auth service; the fake returns a canned token.
pub trait Authenticator {
    fn federated_token(&self) -> impl Future<Output = Result<String, AuthError>> + Send;
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("auth service returned an empty federated token")]
    EmptyToken,
    #[error("federated token is malformed: {0}")]
    MalformedToken(String),
    #[error("auth transport error: {0}")]
    Transport(String),
    #[error("failed to sign auth request: {0}")]
    Signing(String),
}
