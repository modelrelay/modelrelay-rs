//! Minimal Rust SDK for the ModelRelay API.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub const DEFAULT_BASE_URL: &str = "https://api.modelrelay.ai/api/v1";
pub const DEFAULT_CLIENT_HEADER: &str = "modelrelay-rust/0.1";
pub const REQUEST_ID_HEADER: &str = "X-ModelRelay-Chat-Request-Id";
pub const API_KEY_HEADER: &str = "X-ModelRelay-Api-Key";

mod errors;
mod types;

pub use errors::{APIError, Error, FieldError};
pub use types::{
    APIKey, APIKeyCreateRequest, FrontendToken, FrontendTokenRequest, ProxyMessage, ProxyRequest,
    ProxyResponse, StreamEvent, StreamEventKind, Usage,
};

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
pub use client::{ApiKeysClient, AuthClient, Client, Config, LLMClient, ProxyOptions};

#[cfg(all(feature = "client", feature = "streaming"))]
mod sse;
#[cfg(all(feature = "client", feature = "streaming"))]
pub use sse::StreamHandle;
