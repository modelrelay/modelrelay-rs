//! Token providers for automatic token acquisition and caching.
//!
//! This module provides traits and implementations for obtaining and caching
//! customer bearer tokens for use with the ModelRelay API.
//!
//! # Customer Token Provider
//!
//! Mint customer bearer tokens using a secret key:
//!
//! ```ignore
//! use modelrelay::{CustomerTokenProvider, CustomerTokenProviderConfig, CustomerTokenRequest, SecretKey, Client};
//!
//! let provider = CustomerTokenProvider::new(CustomerTokenProviderConfig {
//!     secret_key: SecretKey::parse("mr_sk_...")?,
//!     request: CustomerTokenRequest::for_external_id("user_123"),
//!     base_url: None,
//!     refresh_skew: None,
//!     http_client: None,
//! })?;
//!
//! let client = Client::with_token_provider(provider).build()?;
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::errors::{Error, Result, TransportError, ValidationError};
use crate::types::CustomerTokenRequest;
use crate::{SecretKey, API_KEY_HEADER, DEFAULT_BASE_URL};

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

fn validate_customer_token_request(req: &CustomerTokenRequest) -> Result<()> {
    let has_id = req.customer_id.is_some();
    let has_external = req
        .customer_external_id
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if has_id == has_external {
        return Err(Error::Validation(ValidationError::new(
            "provide exactly one of customer_id or customer_external_id",
        )));
    }
    Ok(())
}

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

/// Configuration for [`CustomerTokenProvider`].
#[derive(Debug, Clone)]
pub struct CustomerTokenProviderConfig {
    /// Base URL for the ModelRelay API.
    pub base_url: Option<String>,
    /// Secret key for authentication (mr_sk_*).
    pub secret_key: SecretKey,
    /// Request payload for /auth/customer-token.
    pub request: CustomerTokenRequest,
    /// How long before expiry to refresh (default: 60s).
    pub refresh_skew: Option<Duration>,
    /// Custom HTTP client.
    pub http_client: Option<reqwest::Client>,
}

/// Token provider that mints customer bearer tokens using a secret key.
pub struct CustomerTokenProvider {
    base_url: String,
    secret_key: SecretKey,
    request: CustomerTokenRequest,
    refresh_skew: Duration,
    http_client: reqwest::Client,
    cache: Arc<Mutex<Option<TokenCache>>>,
}

impl CustomerTokenProvider {
    /// Creates a new customer token provider.
    pub fn new(config: CustomerTokenProviderConfig) -> Result<Self> {
        require_field(config.secret_key.as_str(), "secret_key")?;
        validate_customer_token_request(&config.request)?;

        let base_url = config
            .base_url
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        let http_client = resolve_http_client(config.http_client)?;

        Ok(Self {
            base_url,
            secret_key: config.secret_key,
            request: config.request,
            refresh_skew: config.refresh_skew.unwrap_or(DEFAULT_REFRESH_SKEW),
            http_client,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    async fn mint(&self) -> Result<CustomerTokenResponse> {
        let url = format!(
            "{}/auth/customer-token",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .http_client
            .post(&url)
            .header(API_KEY_HEADER, self.secret_key.as_str())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&self.request)
            .send()
            .await
            .map_err(|e| TransportError::connect("customer token request failed", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::http_failure(
                "customer token request failed",
                status,
                body,
            ));
        }

        resp.json::<CustomerTokenResponse>()
            .await
            .map_err(|e| TransportError::parse_response("customer token response", e))
    }
}

impl TokenProvider for CustomerTokenProvider {
    fn get_token(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>> {
        Box::pin(async move {
            {
                let cache = self.cache.lock().await;
                if let Some(ref cached) = *cache {
                    if cached.is_reusable(self.refresh_skew) {
                        return Ok(cached.token.clone());
                    }
                }
            }

            let token_response = self.mint().await?;
            let token = token_response.token.clone();

            let expires_in = Duration::from_secs(token_response.expires_in as u64);
            let expires_at = Instant::now() + expires_in;

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
}
