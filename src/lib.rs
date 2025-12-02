//! Minimal Rust SDK for the ModelRelay API.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub const DEFAULT_BASE_URL: &str = "https://api.modelrelay.ai/api/v1";
pub const DEFAULT_CLIENT_HEADER: &str = concat!("modelrelay-rust/", env!("CARGO_PKG_VERSION"));
pub const DEFAULT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
pub const REQUEST_ID_HEADER: &str = "X-ModelRelay-Chat-Request-Id";
pub const API_KEY_HEADER: &str = "X-ModelRelay-Api-Key";
pub const STAGING_BASE_URL: &str = "https://api-stg.modelrelay.ai/api/v1";
pub const SANDBOX_BASE_URL: &str = "https://api.sandbox.modelrelay.ai/api/v1";

#[cfg(any(feature = "client", feature = "blocking"))]
mod chat;
#[cfg(feature = "client")]
mod customers;
mod errors;
#[cfg(any(feature = "client", feature = "blocking"))]
mod http;
#[cfg(feature = "mock")]
mod mock;
mod telemetry;
#[cfg(feature = "client")]
mod tiers;
mod types;

#[cfg(any(feature = "client", feature = "blocking"))]
pub use chat::ChatRequestBuilder;
#[cfg(all(feature = "streaming", any(feature = "client", feature = "blocking")))]
pub use chat::ChatStreamAdapter;
#[cfg(any(feature = "client", feature = "blocking", feature = "streaming"))]
pub use errors::{
    APIError, Error, FieldError, RetryMetadata, TransportError, TransportErrorKind, ValidationError,
};
#[cfg(not(any(feature = "client", feature = "blocking", feature = "streaming")))]
pub use errors::{APIError, Error, FieldError, RetryMetadata, ValidationError};
#[cfg(any(feature = "client", feature = "blocking"))]
pub use http::{HeaderEntry, HeaderList, ProxyOptions, RetryConfig, StreamFormat};
#[cfg(feature = "mock")]
pub use mock::{MockAuthClient, MockClient, MockConfig, MockLLMClient, fixtures};
pub use telemetry::{
    HttpRequestMetrics, MetricsCallbacks, RequestContext, StreamFirstTokenMetrics,
    TokenUsageMetrics,
};
pub use types::{
    APIKey, FrontendToken, FrontendTokenRequest, Model, Provider, ProxyMessage, ProxyRequest,
    ProxyRequestBuilder, ProxyResponse, ResponseFormat, ResponseFormatKind, ResponseJSONSchema,
    StopReason, StreamEvent, StreamEventKind, Usage, UsageSummary,
};

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
pub use client::{AuthClient, Client, ClientBuilder, Config, LLMClient};
#[cfg(feature = "client")]
pub use customers::{
    CheckoutSession, CheckoutSessionRequest, Customer, CustomerCreateRequest, CustomerMetadata,
    CustomerUpsertRequest, CustomersClient, SubscriptionStatus,
};
#[cfg(feature = "client")]
pub use tiers::{PriceInterval, Tier, TiersClient};

#[cfg(all(feature = "client", feature = "streaming"))]
mod sse;
#[cfg(all(feature = "client", feature = "streaming"))]
pub use sse::StreamHandle;

#[cfg(feature = "blocking")]
mod blocking;
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use blocking::BlockingProxyHandle;
#[cfg(feature = "blocking")]
pub use blocking::{BlockingAuthClient, BlockingClient, BlockingConfig, BlockingLLMClient};

/// Predefined API environments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Staging,
    Sandbox,
    /// Custom base URL.
    Custom(&'static str),
}

impl Environment {
    pub fn base_url(&self) -> &'static str {
        match self {
            Environment::Production => DEFAULT_BASE_URL,
            Environment::Staging => STAGING_BASE_URL,
            Environment::Sandbox => SANDBOX_BASE_URL,
            Environment::Custom(url) => url,
        }
    }
}
