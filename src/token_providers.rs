//! Token providers for automatic token acquisition and caching.
//!
//! This module provides traits and implementations for obtaining and caching
//! customer bearer tokens for use with the ModelRelay API.
//!
//! # OIDC Exchange
//!
//! Exchange an OIDC id_token from your identity provider (Auth0, Firebase, Google, etc.)
//! for a ModelRelay customer bearer token:
//!
//! ```ignore
//! use modelrelay::{OIDCExchangeTokenProvider, OIDCExchangeConfig, ApiKey, Client};
//!
//! let provider = OIDCExchangeTokenProvider::new(OIDCExchangeConfig {
//!     api_key: ApiKey::parse("mr_pk_...")?,
//!     id_token_source: Box::new(|| Box::pin(async {
//!         Ok(get_id_token_from_provider().await?)
//!     })),
//!     ..Default::default()
//! })?;
//!
//! let client = Client::with_token_provider(provider).build()?;
//! ```
//!
//! # Device Flow
//!
//! For TUI applications and CLIs that can't do browser redirects:
//!
//! ```ignore
//! use modelrelay::{start_device_authorization, poll_device_token, DeviceAuthConfig, DevicePollConfig};
//!
//! let auth = start_device_authorization(DeviceAuthConfig {
//!     device_authorization_endpoint: "https://your-tenant.auth0.com/oauth/device/code".into(),
//!     client_id: "your-client-id".into(),
//!     scope: Some("openid email profile".into()),
//!     ..Default::default()
//! }).await?;
//!
//! println!("Visit {} and enter code: {}", auth.verification_uri, auth.user_code);
//!
//! let token = poll_device_token(DevicePollConfig {
//!     token_endpoint: "https://your-tenant.auth0.com/oauth/token".into(),
//!     client_id: "your-client-id".into(),
//!     device_code: auth.device_code,
//!     interval_seconds: auth.interval_seconds,
//!     deadline: Some(auth.expires_at),
//!     ..Default::default()
//! }).await?;
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::sleep;
use uuid::Uuid;

use crate::errors::{Error, Result, TransportError, ValidationError};
use crate::{ApiKey, API_KEY_HEADER, DEFAULT_BASE_URL};

// Re-export CustomerTokenResponse from generated module (single source of truth)
pub use crate::generated::CustomerTokenResponse;

/// Default refresh skew (60 seconds before expiry).
const DEFAULT_REFRESH_SKEW: Duration = Duration::from_secs(60);

/// Build the default HTTP client with standard timeouts.
///
/// Fails fast if the client cannot be built (e.g., TLS issues).
fn default_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| TransportError::connect("failed to build HTTP client", e))
}

/// Resolve HTTP client from config or build the default.
fn resolve_http_client(custom: Option<reqwest::Client>) -> Result<reqwest::Client> {
    match custom {
        Some(client) => Ok(client),
        None => default_http_client(),
    }
}

/// Validate a required string field, returning trimmed value or error.
fn require_field<'a>(value: &'a str, field_name: &str) -> Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::Validation(ValidationError::new(format!(
            "{} is required",
            field_name
        ))));
    }
    Ok(trimmed)
}

/// Type alias for async id_token provider functions.
pub type IdTokenSource =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<String>> + Send>> + Send + Sync>;

// CustomerTokenResponse is now imported from crate::generated
// This ensures a single source of truth for API response types

/// Trait for token providers that supply bearer tokens for API requests.
///
/// Implementations should handle caching and refresh automatically.
pub trait TokenProvider: Send + Sync {
    /// Returns a valid bearer token, refreshing if necessary.
    fn get_token(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>>;
}

/// Cached token with expiry tracking.
struct TokenCache {
    token: String,
    expires_at: Instant,
}

impl TokenCache {
    fn is_reusable(&self, skew: Duration) -> bool {
        if self.token.is_empty() {
            return false;
        }
        self.expires_at
            .checked_sub(skew)
            .is_some_and(|t| Instant::now() < t)
    }
}

/// Configuration for [`OIDCExchangeTokenProvider`].
pub struct OIDCExchangeConfig {
    /// Base URL for the ModelRelay API.
    pub base_url: Option<String>,
    /// API key for authentication (publishable or secret).
    pub api_key: ApiKey,
    /// Function that provides the OIDC id_token.
    pub id_token_source: IdTokenSource,
    /// Optional project ID override.
    pub project_id: Option<Uuid>,
    /// How long before expiry to refresh (default: 60s).
    pub refresh_skew: Option<Duration>,
    /// Custom HTTP client.
    pub http_client: Option<reqwest::Client>,
}

impl OIDCExchangeConfig {
    /// Creates a new config with required fields.
    pub fn new(api_key: ApiKey, id_token_source: IdTokenSource) -> Self {
        Self {
            base_url: None,
            api_key,
            id_token_source,
            project_id: None,
            refresh_skew: None,
            http_client: None,
        }
    }
}

/// Token provider that exchanges OIDC id_tokens for customer bearer tokens.
///
/// Automatically caches tokens and refreshes them before expiry.
pub struct OIDCExchangeTokenProvider {
    base_url: String,
    api_key: ApiKey,
    id_token_source: IdTokenSource,
    project_id: Option<Uuid>,
    refresh_skew: Duration,
    http_client: reqwest::Client,
    cache: Arc<Mutex<Option<TokenCache>>>,
}

impl OIDCExchangeTokenProvider {
    /// Creates a new OIDC exchange token provider.
    pub fn new(config: OIDCExchangeConfig) -> Result<Self> {
        require_field(config.api_key.as_str(), "api_key")?;

        let base_url = config
            .base_url
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        let http_client = resolve_http_client(config.http_client)?;

        Ok(Self {
            base_url,
            api_key: config.api_key,
            id_token_source: config.id_token_source,
            project_id: config.project_id,
            refresh_skew: config.refresh_skew.unwrap_or(DEFAULT_REFRESH_SKEW),
            http_client,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    async fn exchange(&self) -> Result<CustomerTokenResponse> {
        let id_token = (self.id_token_source)().await?;
        let id_token = require_field(&id_token, "id_token from id_token_source")?;

        #[derive(Serialize)]
        struct ExchangeRequest<'a> {
            id_token: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            project_id: Option<Uuid>,
        }

        let url = format!("{}/auth/oidc/exchange", self.base_url.trim_end_matches('/'));
        let resp = self
            .http_client
            .post(&url)
            .header(API_KEY_HEADER, self.api_key.as_str())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&ExchangeRequest {
                id_token,
                project_id: self.project_id,
            })
            .send()
            .await
            .map_err(|e| TransportError::connect("OIDC exchange request failed", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::http_failure(
                "OIDC exchange failed",
                status,
                body,
            ));
        }

        resp.json::<CustomerTokenResponse>()
            .await
            .map_err(|e| TransportError::parse_response("OIDC exchange response", e))
    }
}

impl TokenProvider for OIDCExchangeTokenProvider {
    fn get_token(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>> {
        Box::pin(async move {
            // Check cache first
            {
                let cache = self.cache.lock().await;
                if let Some(ref cached) = *cache {
                    if cached.is_reusable(self.refresh_skew) {
                        return Ok(cached.token.clone());
                    }
                }
            }

            // Exchange for new token
            let token_response = self.exchange().await?;
            let token = token_response.token.clone();

            // Calculate expiry instant
            let expires_in = Duration::from_secs(token_response.expires_in as u64);
            let expires_at = Instant::now() + expires_in;

            // Cache the token
            {
                let mut cache = self.cache.lock().await;
                *cache = Some(TokenCache {
                    token: token.clone(),
                    expires_at,
                });
            }

            Ok(token)
        })
    }
}

// ============================================================================
// Device Flow
// ============================================================================

/// Configuration for starting OAuth device authorization.
#[derive(Debug, Clone, Default)]
pub struct DeviceAuthConfig {
    /// The OAuth provider's device authorization endpoint.
    pub device_authorization_endpoint: String,
    /// OAuth client ID.
    pub client_id: String,
    /// OAuth scopes (space-separated).
    pub scope: Option<String>,
    /// Optional audience parameter (used by Auth0).
    pub audience: Option<String>,
    /// Custom HTTP client.
    pub http_client: Option<reqwest::Client>,
}

/// Result of device authorization request.
#[derive(Debug, Clone)]
pub struct DeviceAuthorization {
    /// The device code to use when polling.
    pub device_code: String,
    /// The code the user should enter.
    pub user_code: String,
    /// URL the user should visit.
    pub verification_uri: String,
    /// Complete URL with code pre-filled (if provided by provider).
    pub verification_uri_complete: Option<String>,
    /// When the device code expires.
    pub expires_at: Instant,
    /// Suggested polling interval (seconds).
    pub interval_seconds: u64,
}

/// Start OAuth device authorization flow.
///
/// Returns device codes and verification URI for the user to visit.
pub async fn start_device_authorization(config: DeviceAuthConfig) -> Result<DeviceAuthorization> {
    let endpoint = require_field(
        &config.device_authorization_endpoint,
        "device_authorization_endpoint",
    )?;
    let client_id = require_field(&config.client_id, "client_id")?;
    let http = resolve_http_client(config.http_client)?;

    let mut form = vec![("client_id", client_id.to_string())];
    if let Some(scope) = config.scope.as_ref().filter(|s| !s.trim().is_empty()) {
        form.push(("scope", scope.trim().to_string()));
    }
    if let Some(audience) = config.audience.as_ref().filter(|s| !s.trim().is_empty()) {
        form.push(("audience", audience.trim().to_string()));
    }

    let resp = http
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&form)
        .send()
        .await
        .map_err(|e| TransportError::connect("device authorization request failed", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(TransportError::http_failure(
            "device authorization failed",
            status,
            body,
        ));
    }

    #[derive(Deserialize)]
    struct DeviceAuthResponse {
        device_code: String,
        user_code: String,
        verification_uri: Option<String>,
        verification_uri_complete: Option<String>,
        expires_in: u64,
        interval: Option<u64>,
    }

    let payload: DeviceAuthResponse = resp
        .json()
        .await
        .map_err(|e| TransportError::parse_response("device auth response", e))?;

    let verification_uri = payload
        .verification_uri
        .or(payload.verification_uri_complete.clone())
        .ok_or_else(|| {
            Error::Validation(ValidationError::new(
                "device auth response missing verification_uri",
            ))
        })?;

    Ok(DeviceAuthorization {
        device_code: payload.device_code,
        user_code: payload.user_code,
        verification_uri,
        verification_uri_complete: payload.verification_uri_complete,
        expires_at: Instant::now() + Duration::from_secs(payload.expires_in),
        interval_seconds: payload.interval.unwrap_or(5).max(1),
    })
}

/// Configuration for polling the device token endpoint.
#[derive(Debug, Clone, Default)]
pub struct DevicePollConfig {
    /// The OAuth provider's token endpoint.
    pub token_endpoint: String,
    /// OAuth client ID.
    pub client_id: String,
    /// Device code from authorization response.
    pub device_code: String,
    /// Polling interval in seconds.
    pub interval_seconds: u64,
    /// Deadline for polling (defaults to 10 minutes).
    pub deadline: Option<Instant>,
    /// Custom HTTP client.
    pub http_client: Option<reqwest::Client>,
}

/// Result of successful device token poll.
#[derive(Debug, Clone)]
pub struct DeviceToken {
    /// OAuth access token.
    pub access_token: Option<String>,
    /// OIDC id_token.
    pub id_token: Option<String>,
    /// Refresh token (if provided).
    pub refresh_token: Option<String>,
    /// Token type (usually "Bearer").
    pub token_type: Option<String>,
    /// Granted scopes.
    pub scope: Option<String>,
    /// When the token expires.
    pub expires_at: Option<Instant>,
}

/// Poll for OAuth device token until authorized or timeout.
///
/// Handles `authorization_pending` and `slow_down` responses automatically.
pub async fn poll_device_token(config: DevicePollConfig) -> Result<DeviceToken> {
    let endpoint = require_field(&config.token_endpoint, "token_endpoint")?;
    let client_id = require_field(&config.client_id, "client_id")?;
    let device_code = require_field(&config.device_code, "device_code")?;
    let http = resolve_http_client(config.http_client)?;

    let deadline = config
        .deadline
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(600));
    let mut interval = Duration::from_secs(config.interval_seconds.max(1));

    loop {
        if Instant::now() >= deadline {
            return Err(TransportError::timeout("device flow timed out"));
        }

        let form = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
            ("client_id", client_id),
        ];

        let resp = http
            .post(endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&form)
            .send()
            .await
            .map_err(|e| TransportError::connect("device token request failed", e))?;

        #[derive(Deserialize)]
        struct TokenResponse {
            error: Option<String>,
            access_token: Option<String>,
            id_token: Option<String>,
            refresh_token: Option<String>,
            token_type: Option<String>,
            scope: Option<String>,
            expires_in: Option<u64>,
        }

        let payload: TokenResponse = resp
            .json()
            .await
            .map_err(|e| TransportError::parse_response("device token response", e))?;

        if let Some(err) = payload.error.as_deref() {
            match err {
                "authorization_pending" => {
                    sleep(interval).await;
                    continue;
                }
                "slow_down" => {
                    interval += Duration::from_secs(5);
                    sleep(interval).await;
                    continue;
                }
                "expired_token" | "access_denied" | "invalid_grant" => {
                    return Err(TransportError::other(format!(
                        "device flow failed: {}",
                        err
                    )));
                }
                _ => {
                    return Err(TransportError::other(format!("device flow error: {}", err)));
                }
            }
        }

        if payload.access_token.is_none() && payload.id_token.is_none() {
            return Err(TransportError::other(
                "device flow returned invalid token response",
            ));
        }

        let expires_at = payload
            .expires_in
            .map(|secs| Instant::now() + Duration::from_secs(secs));

        return Ok(DeviceToken {
            access_token: payload.access_token.filter(|s| !s.is_empty()),
            id_token: payload.id_token.filter(|s| !s.is_empty()),
            refresh_token: payload.refresh_token.filter(|s| !s.is_empty()),
            token_type: payload.token_type.filter(|s| !s.is_empty()),
            scope: payload.scope.filter(|s| !s.is_empty()),
            expires_at,
        });
    }
}

/// Run complete OAuth device flow and return the id_token.
///
/// Combines [`start_device_authorization`] and [`poll_device_token`] into a single
/// operation, calling the provided callback to display user instructions.
pub async fn run_device_flow_for_id_token<F, Fut>(
    config: DeviceAuthConfig,
    token_endpoint: String,
    on_user_code: F,
) -> Result<String>
where
    F: FnOnce(DeviceAuthorization) -> Fut,
    Fut: Future<Output = ()>,
{
    let auth = start_device_authorization(config.clone()).await?;
    let device_code = auth.device_code.clone();
    let interval = auth.interval_seconds;
    let deadline = auth.expires_at;

    on_user_code(auth).await;

    let token = poll_device_token(DevicePollConfig {
        token_endpoint,
        client_id: config.client_id,
        device_code,
        interval_seconds: interval,
        deadline: Some(deadline),
        http_client: config.http_client,
    })
    .await?;

    token
        .id_token
        .ok_or_else(|| TransportError::other("device flow did not return an id_token"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_cache_not_reusable_when_empty() {
        let cache = TokenCache {
            token: String::new(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        };
        assert!(!cache.is_reusable(Duration::from_secs(60)));
    }

    #[test]
    fn token_cache_not_reusable_when_expired() {
        let cache = TokenCache {
            token: "test".to_string(),
            expires_at: Instant::now() - Duration::from_secs(1),
        };
        assert!(!cache.is_reusable(Duration::from_secs(60)));
    }

    #[test]
    fn token_cache_not_reusable_within_skew() {
        let cache = TokenCache {
            token: "test".to_string(),
            expires_at: Instant::now() + Duration::from_secs(30),
        };
        assert!(!cache.is_reusable(Duration::from_secs(60)));
    }

    #[test]
    fn token_cache_reusable_when_fresh() {
        let cache = TokenCache {
            token: "test".to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        };
        assert!(cache.is_reusable(Duration::from_secs(60)));
    }

    #[test]
    fn oidc_provider_validates_api_key() {
        // ApiKey::parse validates the key format
        let result = ApiKey::parse("");
        assert!(result.is_err());

        let result = ApiKey::parse("invalid-key");
        assert!(result.is_err());

        let result = ApiKey::parse("mr_pk_test123");
        assert!(result.is_ok());
    }
}
