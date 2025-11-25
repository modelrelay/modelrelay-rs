use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(any(feature = "client", feature = "blocking", feature = "streaming"))]
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

/// Structured validation/build error returned by the SDK.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

impl ValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            field: None,
        }
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_error_formats_with_field() {
        let err = ValidationError::new("is required").with_field("model");
        assert_eq!(err.to_string(), "model: is required");
    }

    #[test]
    fn api_error_keeps_status_and_body() {
        let api_err = APIError {
            status: 429,
            code: Some("rate_limit".into()),
            message: "too many requests".into(),
            request_id: Some("req_123".into()),
            fields: Vec::new(),
            retries: Some(RetryMetadata {
                attempts: 2,
                last_status: Some(429),
                last_error: None,
            }),
            raw_body: Some("{\"error\":\"rate limit\"}".into()),
        };

        assert_eq!(api_err.to_string(), "rate_limit (429): too many requests");
        assert_eq!(api_err.status, 429);
        assert!(api_err.raw_body.is_some());
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(field) = &self.field {
            write!(f, "{}: {}", field, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for ValidationError {}

impl From<String> for ValidationError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for ValidationError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
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
    /// Raw response body for debugging (when available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_body: Option<String>,
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
            raw_body: None,
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
#[cfg(any(feature = "client", feature = "blocking", feature = "streaming"))]
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
#[cfg(any(feature = "client", feature = "blocking", feature = "streaming"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransportErrorKind {
    Timeout,
    Connect,
    Request,
    Other,
}

#[cfg(any(feature = "client", feature = "blocking", feature = "streaming"))]
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
    Validation(#[from] ValidationError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Api(#[from] APIError),

    #[cfg(any(feature = "client", feature = "blocking", feature = "streaming"))]
    #[error("{0}")]
    Transport(#[from] TransportError),

    #[error("stream backpressure: dropped {dropped} events")]
    StreamBackpressure { dropped: usize },

    #[error("stream closed")]
    StreamClosed,
}
