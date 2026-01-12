use std::{fmt, time::Duration};

use serde::{Deserialize, Serialize};

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

/// Workflow semantic validation error returned by the API compiler (`/workflows/compile`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowValidationIssue {
    pub code: String,
    pub path: String,
    pub message: String,
}

/// Typed workflow validation error surfaced by the SDK.
///
/// Note: `request_id` and `retries` are populated from HTTP metadata when available;
/// they are not part of the server's JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowValidationError {
    pub issues: Vec<WorkflowValidationIssue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<RetryMetadata>,
}

impl fmt::Display for WorkflowValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.issues.is_empty() {
            return write!(f, "workflow validation error");
        }
        let first = &self.issues[0];
        if self.issues.len() == 1 {
            return write!(f, "{}: {}", first.path, first.message);
        }
        write!(
            f,
            "{}: {} (and {} more)",
            first.path,
            first.message,
            self.issues.len() - 1
        )
    }
}

impl std::error::Error for WorkflowValidationError {}

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

/// API error codes returned by the server (internal use).
mod error_codes {
    pub const NOT_FOUND: &str = "NOT_FOUND";
    pub const VALIDATION_ERROR: &str = "VALIDATION_ERROR";
    pub const RATE_LIMIT: &str = "RATE_LIMIT";
    pub const UNAUTHORIZED: &str = "UNAUTHORIZED";
    pub const FORBIDDEN: &str = "FORBIDDEN";
    pub const SERVICE_UNAVAILABLE: &str = "SERVICE_UNAVAILABLE";
    pub const INVALID_INPUT: &str = "INVALID_INPUT";
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

    /// Returns true if the error is a not found error.
    pub fn is_not_found(&self) -> bool {
        self.code.as_deref() == Some(error_codes::NOT_FOUND)
    }

    /// Returns true if the error is a validation error.
    pub fn is_validation(&self) -> bool {
        matches!(
            self.code.as_deref(),
            Some(error_codes::VALIDATION_ERROR) | Some(error_codes::INVALID_INPUT)
        )
    }

    /// Returns true if the error is a rate limit error.
    pub fn is_rate_limit(&self) -> bool {
        self.code.as_deref() == Some(error_codes::RATE_LIMIT)
    }

    /// Returns true if the error is an unauthorized error.
    pub fn is_unauthorized(&self) -> bool {
        self.code.as_deref() == Some(error_codes::UNAUTHORIZED)
    }

    /// Returns true if the error is a forbidden error.
    pub fn is_forbidden(&self) -> bool {
        self.code.as_deref() == Some(error_codes::FORBIDDEN)
    }

    /// Returns true if the error is a service unavailable error.
    pub fn is_unavailable(&self) -> bool {
        self.code.as_deref() == Some(error_codes::SERVICE_UNAVAILABLE)
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransportErrorKind {
    Timeout,
    Connect,
    Request,
    /// Response was received but contained no content.
    EmptyResponse,
    Other,
}

impl TransportError {
    /// Create a connection error from a reqwest error.
    pub fn connect(context: impl Into<String>, source: reqwest::Error) -> Error {
        Error::Transport(Self {
            kind: TransportErrorKind::Connect,
            message: format!("{}: {}", context.into(), source),
            source: Some(source),
            retries: None,
        })
    }

    /// Create a transport error without a source.
    pub fn other(message: impl Into<String>) -> Error {
        Error::Transport(Self {
            kind: TransportErrorKind::Other,
            message: message.into(),
            source: None,
            retries: None,
        })
    }

    /// Create an error from a failed HTTP response.
    pub fn http_failure(
        context: impl Into<String>,
        status: reqwest::StatusCode,
        body: String,
    ) -> Error {
        Error::Transport(Self {
            kind: TransportErrorKind::Other,
            message: format!("{} ({}): {}", context.into(), status, body),
            source: None,
            retries: None,
        })
    }

    /// Create an error from a JSON parse failure.
    pub fn parse_response(context: impl Into<String>, source: reqwest::Error) -> Error {
        Error::Transport(Self {
            kind: TransportErrorKind::Other,
            message: format!("failed to parse {}: {}", context.into(), source),
            source: Some(source),
            retries: None,
        })
    }

    /// Create a timeout error.
    pub fn timeout(message: impl Into<String>) -> Error {
        Error::Transport(Self {
            kind: TransportErrorKind::Timeout,
            message: message.into(),
            source: None,
            retries: None,
        })
    }
}

impl fmt::Display for TransportErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            TransportErrorKind::Timeout => "timeout",
            TransportErrorKind::Connect => "connect",
            TransportErrorKind::Request => "request",
            TransportErrorKind::EmptyResponse => "empty response",
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

    #[error("{0}")]
    WorkflowValidation(#[from] WorkflowValidationError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Api(#[from] APIError),

    #[error("{0}")]
    Transport(#[from] TransportError),

    #[error("stream backpressure: dropped {dropped} events")]
    StreamBackpressure { dropped: usize },

    #[error("stream closed")]
    StreamClosed,

    #[cfg(feature = "streaming")]
    #[error("stream protocol error: {message}")]
    StreamProtocol {
        message: String,
        /// The raw data that failed to parse (truncated for logging).
        raw_data: Option<String>,
    },

    #[cfg(feature = "streaming")]
    #[error("expected NDJSON stream ({expected}), got Content-Type {received}")]
    StreamContentType {
        expected: &'static str,
        received: String,
        status: u16,
    },

    #[cfg(feature = "streaming")]
    #[error("{0}")]
    StreamTimeout(#[from] StreamTimeoutError),

    #[error("agent exceeded maximum turns ({max_turns}) without completing")]
    AgentMaxTurns { max_turns: usize },
}

/// Which streaming timeout triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamTimeoutKind {
    Ttft,
    Idle,
    Total,
}

impl fmt::Display for StreamTimeoutKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamTimeoutKind::Ttft => write!(f, "ttft"),
            StreamTimeoutKind::Idle => write!(f, "idle"),
            StreamTimeoutKind::Total => write!(f, "total"),
        }
    }
}

/// Typed error for streaming timeouts.
#[derive(Debug, Error, Clone)]
#[error("stream {kind} timeout after {timeout:?}")]
pub struct StreamTimeoutError {
    pub kind: StreamTimeoutKind,
    pub timeout: Duration,
}
