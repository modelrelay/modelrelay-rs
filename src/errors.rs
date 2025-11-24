use std::fmt;

use serde::{Deserialize, Serialize};
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
}

impl APIError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            code: None,
            message: message.into(),
            request_id: None,
            fields: Vec::new(),
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

/// Unified error type surfaced by the SDK.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Config(String),

    #[cfg(feature = "client")]
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Api(#[from] APIError),

    #[error("stream closed")]
    StreamClosed,
}
