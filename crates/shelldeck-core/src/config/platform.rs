//! Typed, client-only access to the shared Automonique platform contract.
//!
//! ShellDeck renders platform state and may request explicit attach/control
//! operations. It does not define wire types, execute provider jobs, or own a
//! runtime loop.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use automonique_platform_client::{BearerToken, HttpsTransport, PlatformClient};
pub use automonique_protocol::platform::{
    ActionReceipt, Attachment, Capabilities, ClientId, ControlLease, ControlLeaseId,
    FreshnessState, IdempotencyKey, PlatformCursor, PlatformMethod, PlatformTransport,
    ReleaseControlRequest, ResourceAuthority, ResourceCoordinate, ResourceKind, ResourceRecord,
    SessionRecord,
};
use url::Url;

use crate::error::{Result, ShellDeckError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformSnapshot {
    pub capabilities: Capabilities,
    pub resources: Vec<ResourceRecord>,
    pub cursor: PlatformCursor,
    pub sessions: Vec<SessionRecord>,
    pub sessions_cursor: PlatformCursor,
}

#[derive(Clone, Eq, PartialEq)]
pub struct PlatformConnection {
    endpoint: String,
    token: String,
}

impl fmt::Debug for PlatformConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlatformConnection")
            .field("endpoint", &self.endpoint)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl PlatformConnection {
    pub fn new(dashboard_url: &str, token: &str) -> Result<Self> {
        let endpoint = platform_endpoint(dashboard_url)?;
        BearerToken::new(token.to_owned()).map_err(platform_error)?;
        Ok(Self {
            endpoint,
            token: token.to_owned(),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn client(&self) -> Result<PlatformClient<HttpsTransport>> {
        let token = BearerToken::new(self.token.clone()).map_err(platform_error)?;
        let transport =
            HttpsTransport::new(self.endpoint.clone(), token).map_err(platform_error)?;
        Ok(PlatformClient::new(transport))
    }

    pub fn snapshot(&self) -> Result<PlatformSnapshot> {
        let mut client = self.client()?;
        let capabilities = client.capabilities().map_err(platform_error)?;
        let resources = client.snapshot(Vec::new()).map_err(platform_error)?;
        let sessions = client
            .list_sessions(ResourceAuthority::Automonique, None)
            .map_err(platform_error)?;
        Ok(PlatformSnapshot {
            capabilities,
            resources: resources.resources,
            cursor: resources.cursor,
            sessions: sessions.sessions,
            sessions_cursor: sessions.cursor,
        })
    }

    pub fn attach(&self, session: ResourceCoordinate, client: ClientId) -> Result<Attachment> {
        self.client()?
            .attach(session, client)
            .map_err(platform_error)
    }

    pub fn detach(&self, session: ResourceCoordinate, client: ClientId) -> Result<()> {
        self.client()?
            .detach(session, client)
            .map_err(platform_error)
    }

    pub fn claim_control(
        &self,
        session: ResourceCoordinate,
        client: ClientId,
    ) -> Result<ControlLease> {
        self.client()?
            .claim_control(session, client, unique_key("claim"))
            .map_err(platform_error)
    }

    pub fn release_control(
        &self,
        session: ResourceCoordinate,
        client: ClientId,
        lease: ControlLeaseId,
    ) -> Result<()> {
        self.client()?
            .release_control(ReleaseControlRequest {
                session,
                client,
                lease,
                idempotency_key: unique_key("release"),
            })
            .map_err(platform_error)
    }
}

pub fn platform_endpoint(dashboard_url: &str) -> Result<String> {
    let url = Url::parse(dashboard_url.trim())
        .map_err(|_| ShellDeckError::Connection("platform dashboard URL is invalid".to_string()))?;
    if url.scheme() != "https"
        && !(cfg!(test)
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
    {
        return Err(ShellDeckError::Connection(
            "platform dashboard must use HTTPS".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
        return Err(ShellDeckError::Connection(
            "platform dashboard URL contains unsupported components".to_string(),
        ));
    }
    let origin = url.origin().ascii_serialization();
    Ok(format!("{origin}/api/platform"))
}

pub fn stable_client_id(seed: &str) -> Result<ClientId> {
    let normalized = seed
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(96)
        .collect::<String>();
    ClientId::new(format!(
        "shelldeck-{}",
        if normalized.is_empty() {
            "desktop"
        } else {
            &normalized
        }
    ))
    .map_err(|_| ShellDeckError::Connection("platform client identity is invalid".to_string()))
}

fn unique_key(operation: &str) -> IdempotencyKey {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    IdempotencyKey::new(format!("shelldeck-{operation}-{nanos}"))
        .expect("bounded generated platform idempotency key")
}

fn platform_error(error: automonique_platform_client::ClientError) -> ShellDeckError {
    ShellDeckError::Connection(format!("platform request refused: {}", error.category()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_canonical_and_https_only() {
        assert_eq!(
            platform_endpoint("https://monique.example.test/dashboard").unwrap(),
            "https://monique.example.test/api/platform"
        );
        assert!(platform_endpoint("http://monique.example.test/").is_err());
        assert!(platform_endpoint("https://user@monique.example.test/").is_err());
    }

    #[test]
    fn client_identity_is_stable_bounded_and_typed() {
        let client = stable_client_id("desktop 01/@example").unwrap();
        assert_eq!(client.as_str(), "shelldeck-desktop01example");
    }

    #[test]
    fn connection_debug_never_contains_the_bearer() {
        let connection =
            PlatformConnection::new("https://monique.example.test/", "fixture-sensitive-token")
                .unwrap();
        let rendered = format!("{connection:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("fixture-sensitive-token"));
    }
}
