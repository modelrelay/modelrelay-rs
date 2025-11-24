use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(any(feature = "client", feature = "blocking"))]
use reqwest;

/// Retry metadata surfaced on transport/API errors when retries were attempted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryMetadata {
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}
use thiserror::Error;

/// Field-level validation error returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldError {
    pub field: Option<String>,
    pub message: String,
}

/// Structured error envelope returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct APIError {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
    pub request_id: Option<String>,
    #[serde(default)]
    pub fields: Vec<FieldError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<RetryMetadata>,
}

impl APIError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            code: None,
            message: message.into(),
            request_id: None,
            fields: Vec::new(),
            retries: None,
        }
    }
}

impl fmt::Display for APIError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = &self.code {
            write!(f, "{} ({}): {}", code, self.status, self.message)
        } else {
            write!(f, "{}: {}", self.status, self.message)
        }
    }
}

impl std::error::Error for APIError {}

/// Convenience alias for fallible SDK results.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Transport-level error (timeouts, DNS/TLS/connectivity).
#[cfg(any(feature = "client", feature = "blocking"))]
#[derive(Debug, Error)]
#[error("{kind}: {message}")]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub message: String,
    #[source]
    pub source: Option<reqwest::Error>,
    pub retries: Option<RetryMetadata>,
}

/// Broad transport error kinds for classification.
#[cfg(any(feature = "client", feature = "blocking"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransportErrorKind {
    Timeout,
    Connect,
    Request,
    Other,
}

#[cfg(any(feature = "client", feature = "blocking"))]
impl fmt::Display for TransportErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            TransportErrorKind::Timeout => "timeout",
            TransportErrorKind::Connect => "connect",
            TransportErrorKind::Request => "request",
            TransportErrorKind::Other => "transport",
        };
        write!(f, "{label}")
    }
}

/// Unified error type surfaced by the SDK.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Config(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Api(#[from] APIError),

    #[cfg(any(feature = "client", feature = "blocking"))]
    #[error("{0}")]
    Transport(#[from] TransportError),

    #[error("stream backpressure: dropped {dropped} events")]
    StreamBackpressure { dropped: usize },

    #[error("stream closed")]
    StreamClosed,
}
