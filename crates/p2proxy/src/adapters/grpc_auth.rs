use proxy_core::ports::AuthError;
use proxy_core::ports::{Authenticator, Identity};
use protocols::auth::v1::{
    FederatedApiTokenAuthRequest, authentication_service_client::AuthenticationServiceClient,
};
use sha2::{Digest, Sha256};
use tonic::codec::CompressionEncoding;
use tonic::transport::Channel;
use tracing::info;

use super::keypair_identity::KeypairIdentity;

/// Exchanges the Bitping API key for a federated PASETO over gRPC.
pub struct GrpcAuth {
    channel: Channel,
    api_key: String,
    identity: KeypairIdentity,
}

impl GrpcAuth {
    pub fn new(channel: Channel, api_key: String, identity: KeypairIdentity) -> Self {
        Self {
            channel,
            api_key,
            identity,
        }
    }
}

impl Authenticator for GrpcAuth {
    async fn federated_token(&self) -> Result<String, AuthError> {
        let mut client = AuthenticationServiceClient::new(self.channel.clone())
            .send_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Gzip);

        let digest = Sha256::digest(self.api_key.as_bytes());
        let signature = self.identity.sign(digest.as_slice())?;
        let signature_b58 = base58_monero::encode_check(&signature)
            .map_err(|e| AuthError::Signing(e.to_string()))?;
        let public_key = self.identity.public_b58()?;

        let response = client
            .federated_api_token_authenticate(tonic::Request::new(FederatedApiTokenAuthRequest {
                api_token: self.api_key.clone(),
                signature: signature_b58,
                public_key,
            }))
            .await
            .map_err(|e| AuthError::Transport(e.to_string()))?;

        let token = response.into_inner().token;
        sanity_check_federated_token(&token)?;
        Ok(token)
    }
}

/// Shape-only check on the federated PASETO so a malfunctioning auth service
/// fails fast rather than producing hours of unattributable reports. The hub
/// does the trusted cryptographic validation at ingestion.
fn sanity_check_federated_token(token: &str) -> Result<(), AuthError> {
    if token.is_empty() {
        return Err(AuthError::EmptyToken);
    }
    const V4_PUBLIC_PREFIX: &str = "v4.public.";
    if !token.starts_with(V4_PUBLIC_PREFIX) {
        return Err(AuthError::MalformedToken(format!(
            "expected `{V4_PUBLIC_PREFIX}` prefix, got `{}...`",
            token.chars().take(16).collect::<String>()
        )));
    }
    let dot_count = token.bytes().filter(|&b| b == b'.').count();
    if dot_count < 2 {
        return Err(AuthError::MalformedToken(format!(
            "only {dot_count} `.` separator(s); expected at least 2 for PASETO v4.public"
        )));
    }
    info!(token_len = token.len(), "federated PASETO acquired");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_token() {
        assert!(matches!(
            sanity_check_federated_token(""),
            Err(AuthError::EmptyToken)
        ));
    }

    #[test]
    fn rejects_wrong_version() {
        assert!(matches!(
            sanity_check_federated_token("v3.local.something"),
            Err(AuthError::MalformedToken(_))
        ));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            sanity_check_federated_token("not-a-paseto"),
            Err(AuthError::MalformedToken(_))
        ));
    }

    #[test]
    fn accepts_well_formed_v4_public() {
        sanity_check_federated_token("v4.public.eyJzb21lIjoiY2xhaW1zIn0.eyJmb290ZXIiOiJoZXJlIn0")
            .expect("well-formed token passes");
    }
}
