use std::time::Duration;

use reqwest::{header::HeaderMap, Method, StatusCode};

use crate::{
    errors::{APIError, Error, RetryMetadata},
    REQUEST_ID_HEADER,
};

/// Optional headers for proxy calls.
#[derive(Clone, Default)]
pub struct ProxyOptions {
    pub request_id: Option<String>,
    pub headers: HeaderList,
    pub timeout: Option<Duration>,
    pub retry: Option<RetryConfig>,
}

impl ProxyOptions {
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .push(HeaderEntry::new(key.into(), value.into()));
        self
    }

    /// Override the overall request timeout for this call.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Override the retry policy for this call.
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Disable retries for this call.
    pub fn disable_retry(mut self) -> Self {
        self.retry = Some(RetryConfig::disabled());
        self
    }
}

// StreamFormat enum removed - unified NDJSON is the only streaming format

/// Retry/backoff configuration (defaults use 3 attempts + jittered exponential backoff).
#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub retry_post: bool,
}

impl RetryConfig {
    pub fn disabled() -> Self {
        Self {
            max_attempts: 1,
            ..Default::default()
        }
    }

    /// Whether the given status code should trigger a retry for this method.
    pub fn should_retry_status(&self, method: &Method, status: StatusCode) -> bool {
        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::REQUEST_TIMEOUT {
            return self.allow_for_method(method);
        }
        if status.is_server_error() {
            return self.allow_for_method(method);
        }
        false
    }

    /// Whether the given transport error should trigger a retry.
    pub fn should_retry_error(&self, method: &Method, err: &reqwest::Error) -> bool {
        if err.is_timeout() || err.is_connect() || err.is_request() {
            return self.allow_for_method(method);
        }
        false
    }

    /// Jittered exponential backoff for the given attempt (1-indexed).
    pub fn backoff_delay(&self, attempt: u32) -> Duration {
        let exp = if attempt == 0 {
            0
        } else {
            (attempt - 1).min(10)
        };
        let base = self.base_backoff.saturating_mul(2u32.saturating_pow(exp));
        let capped = std::cmp::min(base, self.max_backoff);
        let jitter = 0.5 + fastrand::f64(); // 0.5x .. 1.5x
        let seconds = (capped.as_secs_f64() * jitter).min(self.max_backoff.as_secs_f64());
        Duration::from_secs_f64(seconds)
    }

    fn allow_for_method(&self, method: &Method) -> bool {
        if method == Method::POST {
            return self.retry_post;
        }
        true
    }
}

/// Structured header/metadata list with validation.
#[derive(Clone, Debug, Default)]
pub struct HeaderList(Vec<HeaderEntry>);

impl HeaderList {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Add a header entry. Panics if key or value is empty/whitespace-only.
    ///
    /// # Panics
    /// Panics if the header key or value is empty or contains only whitespace.
    /// This is a fail-fast behavior to catch configuration errors early.
    pub fn push(&mut self, entry: HeaderEntry) {
        assert!(
            entry.is_valid(),
            "Invalid header: key and value must be non-empty (got key={:?}, value={:?})",
            entry.key,
            entry.value
        );
        self.0.push(entry);
    }

    pub fn iter(&self) -> impl Iterator<Item = &HeaderEntry> {
        self.0.iter()
    }
}

#[derive(Clone, Debug)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
}

impl HeaderEntry {
    pub fn new(key: String, value: String) -> Self {
        Self { key, value }
    }

    pub fn is_valid(&self) -> bool {
        !(self.key.trim().is_empty() || self.value.trim().is_empty())
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_backoff: Duration::from_millis(300),
            max_backoff: Duration::from_secs(5),
            retry_post: true,
        }
    }
}

pub(crate) fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(REQUEST_ID_HEADER) {
        if let Ok(s) = value.to_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(value) = headers.get("X-Request-Id") {
        if let Ok(s) = value.to_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

pub(crate) fn parse_api_error_parts(
    status: StatusCode,
    headers: &HeaderMap,
    body: String,
    retries: Option<RetryMetadata>,
) -> Error {
    let request_id = request_id_from_headers(headers);
    let status_code = status.as_u16();
    let status_text = status
        .canonical_reason()
        .unwrap_or("request failed")
        .to_string();

    if body.is_empty() {
        return APIError {
            status: status_code,
            code: None,
            message: status_text,
            request_id,
            fields: Vec::new(),
            retries,
            raw_body: None,
        }
        .into();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(err_obj) = value.get("error").and_then(|v| v.as_object()) {
            let code = err_obj
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let message = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| status_text.clone());
            let fields = err_obj
                .get("fields")
                .and_then(|v| {
                    serde_json::from_value::<Vec<crate::errors::FieldError>>(v.clone()).ok()
                })
                .unwrap_or_default();
            let status_override = err_obj
                .get("status")
                .and_then(|v| v.as_u64())
                .map(|v| v as u16)
                .unwrap_or(status_code);
            return APIError {
                status: status_override,
                code,
                message,
                request_id: value
                    .get("request_id")
                    .or_else(|| value.get("requestId"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or(request_id),
                fields,
                retries,
                raw_body: Some(body.clone()),
            }
            .into();
        }

        if let Some(message) = value.get("message").and_then(|v| v.as_str()) {
            let code = value
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let fields = value
                .get("fields")
                .and_then(|v| {
                    serde_json::from_value::<Vec<crate::errors::FieldError>>(v.clone()).ok()
                })
                .unwrap_or_default();
            let req_id = value
                .get("request_id")
                .or_else(|| value.get("requestId"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or(request_id);
            return APIError {
                status: status_code,
                code,
                message: message.to_string(),
                request_id: req_id,
                fields,
                retries,
                raw_body: Some(body.clone()),
            }
            .into();
        }
    }

    APIError {
        status: status_code,
        code: None,
        message: body.clone(),
        request_id,
        fields: Vec::new(),
        retries,
        raw_body: Some(body),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_respects_max_and_jitter() {
        let retry = RetryConfig {
            max_attempts: 3,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(1),
            retry_post: true,
        };

        let delay = retry.backoff_delay(5);
        assert!(delay <= Duration::from_secs(1));
        assert!(delay >= Duration::from_millis(250));
    }

    #[test]
    fn retry_post_toggle_honored() {
        let retry = RetryConfig {
            retry_post: false,
            ..Default::default()
        };
        assert!(!retry.should_retry_status(&Method::POST, StatusCode::INTERNAL_SERVER_ERROR));
        assert!(retry.should_retry_status(&Method::GET, StatusCode::INTERNAL_SERVER_ERROR));
    }

    #[test]
    fn proxy_options_disable_retry_sets_single_attempt() {
        let opts = ProxyOptions::default().disable_retry();
        assert_eq!(opts.retry.unwrap().max_attempts, 1);
    }

    #[test]
    fn header_list_accepts_valid_entries() {
        let mut list = HeaderList::new();
        list.push(HeaderEntry::new(
            "X-Custom".to_string(),
            "value".to_string(),
        ));
        assert_eq!(list.iter().count(), 1);
    }

    #[test]
    #[should_panic(expected = "Invalid header")]
    fn header_list_panics_on_empty_key() {
        let mut list = HeaderList::new();
        list.push(HeaderEntry::new("".to_string(), "value".to_string()));
    }

    #[test]
    #[should_panic(expected = "Invalid header")]
    fn header_list_panics_on_empty_value() {
        let mut list = HeaderList::new();
        list.push(HeaderEntry::new("key".to_string(), "".to_string()));
    }

    #[test]
    #[should_panic(expected = "Invalid header")]
    fn header_list_panics_on_whitespace_only() {
        let mut list = HeaderList::new();
        list.push(HeaderEntry::new("   ".to_string(), "value".to_string()));
    }
}
