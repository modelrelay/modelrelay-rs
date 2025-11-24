//! Minimal Rust SDK for the ModelRelay API.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub const DEFAULT_BASE_URL: &str = "https://api.modelrelay.ai/api/v1";
pub const DEFAULT_CLIENT_HEADER: &str = concat!("modelrelay-rust/", env!("CARGO_PKG_VERSION"));
pub const DEFAULT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
pub const REQUEST_ID_HEADER: &str = "X-ModelRelay-Chat-Request-Id";
pub const API_KEY_HEADER: &str = "X-ModelRelay-Api-Key";

mod errors;
#[cfg(any(feature = "client", feature = "blocking"))]
mod http;
mod types;

pub use errors::{APIError, Error, FieldError, RetryMetadata, TransportError, TransportErrorKind};
#[cfg(any(feature = "client", feature = "blocking"))]
pub use http::{ProxyOptions, RetryConfig};
pub use types::{
    APIKey, APIKeyCreateRequest, FrontendToken, FrontendTokenRequest, ProxyMessage, ProxyRequest,
    ProxyResponse, StreamEvent, StreamEventKind, Usage,
};

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
pub use client::{ApiKeysClient, AuthClient, Client, Config, LLMClient};

#[cfg(all(feature = "client", feature = "streaming"))]
mod sse;
#[cfg(all(feature = "client", feature = "streaming"))]
pub use sse::StreamHandle;

#[cfg(feature = "blocking")]
mod blocking;
#[cfg(feature = "blocking")]
pub use blocking::{
    BlockingApiKeysClient, BlockingAuthClient, BlockingClient, BlockingConfig, BlockingLLMClient,
};
