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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
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

/// Outcome of a polling attempt.
#[derive(Debug)]
pub enum PollResult<T> {
    /// Polling completed with a value.
    Ready(T),
    /// Polling should continue, optionally with a new delay.
    Pending { retry_after: Option<Duration> },
}

/// Configuration for polling with backoff.
#[derive(Debug, Clone)]
pub struct PollUntilOptions {
    /// Default polling interval.
    pub interval: Duration,
    /// Absolute deadline for polling.
    pub deadline: Instant,
    /// Error message when the deadline is exceeded.
    pub timeout_message: String,
}

/// Poll until the closure returns a value or the deadline is exceeded.
pub async fn poll_until<T, F, Fut>(opts: PollUntilOptions, mut poll: F) -> Result<T>
where
    F: FnMut(u32, Duration) -> Fut,
    Fut: Future<Output = Result<PollResult<T>>>,
{
    let mut interval = if opts.interval.is_zero() {
        Duration::from_millis(1)
    } else {
        opts.interval
    };
    let mut attempt: u32 = 0;
    loop {
        if Instant::now() >= opts.deadline {
            return Err(TransportError::timeout(opts.timeout_message));
        }
        match poll(attempt, interval).await? {
            PollResult::Ready(value) => return Ok(value),
            PollResult::Pending { retry_after } => {
                let delay = retry_after.unwrap_or(interval);
                interval = if delay.is_zero() {
                    Duration::from_millis(1)
                } else {
                    delay
                };
                sleep(interval).await;
                attempt += 1;
            }
        }
    }
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
