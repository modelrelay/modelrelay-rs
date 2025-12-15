//! Shared runtime-agnostic logic for async and blocking clients.
//!
//! This module contains data structures and pure functions that are used by both
//! the async client (`client.rs`) and the blocking client (`blocking.rs`).
//!
//! ## Contents
//!
//! - **Retry**: [`RetryState`] for tracking HTTP retry attempts
//! - **Auth Validation**: [`validate_secret_key`], [`validate_api_key`] for key type checks
//! - **NDJSON Parsing**: [`RawEvent`], [`consume_ndjson_buffer`], [`map_event`] for streaming

use std::time::Duration;

use reqwest::{Method, StatusCode};

#[cfg(feature = "streaming")]
use crate::errors::Result;
use crate::errors::{Error, RetryMetadata, ValidationError};
use crate::http::RetryConfig;
#[cfg(feature = "streaming")]
use crate::types::{
    Model, StopReason, StreamEvent, StreamEventKind, ToolCall, ToolCallDelta, Usage,
};
use crate::ApiKey;

/// Tracks retry state across attempts for both async and blocking clients.
#[derive(Default)]
pub(crate) struct RetryState {
    pub(crate) attempts: u32,
    pub(crate) last_status: Option<u16>,
    pub(crate) last_error: Option<String>,
}

impl RetryState {
    pub(crate) fn new() -> Self {
        Self {
            attempts: 0,
            last_status: None,
            last_error: None,
        }
    }

    pub(crate) fn record_attempt(&mut self, attempt: u32) {
        self.attempts = attempt;
    }

    pub(crate) fn record_status(&mut self, status: StatusCode) {
        self.last_status = Some(status.as_u16());
    }

    pub(crate) fn record_error(&mut self, err: &reqwest::Error) {
        self.last_error = Some(err.to_string());
    }

    pub(crate) fn metadata(&self) -> Option<RetryMetadata> {
        if self.attempts <= 1 {
            None
        } else {
            Some(RetryMetadata {
                attempts: self.attempts,
                last_status: self.last_status,
                last_error: self.last_error.clone(),
            })
        }
    }
}

/// API key validation result.
pub(crate) type KeyResult = std::result::Result<(), Error>;

/// Validates that an API key is present and is a secret key (mr_sk_*).
///
/// Used for privileged operations like customer management, tier checkout, etc.
pub(crate) fn validate_secret_key(api_key: &Option<ApiKey>) -> KeyResult {
    match api_key {
        Some(ApiKey::Secret(_)) => Ok(()),
        Some(_) => Err(Error::Validation(ValidationError::new(
            "secret key (mr_sk_*) required for this operation",
        ))),
        None => Err(Error::Validation(ValidationError::new(
            "API key is required",
        ))),
    }
}

/// Validates that an API key is present and is either publishable (mr_pk_*) or secret (mr_sk_*).
///
/// Used for operations that work with both key types (tier listing, customer claim, etc.).
pub(crate) fn validate_api_key(api_key: &Option<ApiKey>) -> KeyResult {
    match api_key {
        Some(_) => Ok(()),
        None => Err(Error::Validation(ValidationError::new(
            "API key is required",
        ))),
    }
}

/// Checks if a token looks like a JWT (3 base64url segments separated by '.').
///
/// Returns `false` for ModelRelay API keys (mr_sk_*, mr_pk_*) even if passed as bearer tokens.
/// Used to determine if model validation should be skipped (JWTs have models baked in).
pub(crate) fn is_jwt_token(token: &str) -> bool {
    let t = token.trim();
    if t.is_empty() {
        return false;
    }
    // Treat API keys passed as bearer tokens as non-JWT for model validation.
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("mr_sk_") || lower.starts_with("mr_pk_") {
        return false;
    }
    // JWTs have 3 base64url segments separated by '.'.
    t.matches('.').count() >= 2
}

// ============================================================================
// Shared Client Configuration
// ============================================================================

/// Shared client configuration used by both async and blocking clients.
///
/// This struct contains all configuration that is runtime-agnostic and can be
/// shared between async (`Client`) and blocking (`BlockingClient`) implementations.
/// The actual HTTP client instance is stored separately since it differs between
/// async (`reqwest::Client`) and blocking (`reqwest::blocking::Client`).
#[allow(dead_code)] // Prepared for future async/blocking unification
#[derive(Clone)]
pub(crate) struct ClientConfig {
    /// Base URL for API requests.
    pub base_url: reqwest::Url,
    /// API key for authentication (mr_sk_* or mr_pk_*).
    pub api_key: Option<crate::ApiKey>,
    /// Bearer token for customer-scoped requests.
    pub access_token: Option<String>,
    /// Custom client header for identification.
    pub client_header: Option<String>,
    /// Request timeout duration.
    pub request_timeout: Duration,
    /// Retry configuration for failed requests.
    pub retry: RetryConfig,
    /// Default headers to include on all requests.
    pub default_headers: Option<crate::http::HeaderList>,
}

#[allow(dead_code)] // Methods prepared for future use
impl ClientConfig {
    /// Checks if authentication is configured.
    pub fn has_auth(&self) -> bool {
        self.api_key.is_some() || self.access_token.is_some()
    }

    /// Validates that authentication is configured.
    pub fn ensure_auth(&self) -> std::result::Result<(), Error> {
        if self.has_auth() {
            Ok(())
        } else {
            Err(Error::Validation(ValidationError::new(
                "api key or access token is required",
            )))
        }
    }

    /// Checks if the access token looks like a JWT.
    pub fn has_jwt_access_token(&self) -> bool {
        self.access_token
            .as_deref()
            .map(is_jwt_token)
            .unwrap_or(false)
    }
}

// ============================================================================
// Retry Decision Logic (Pure)
// ============================================================================

/// Result of evaluating a single HTTP attempt for retry decisions.
///
/// This captures the outcome of an HTTP request in a runtime-agnostic way,
/// allowing pure retry decision logic to operate on the result.
#[allow(dead_code)] // Used in tests; ready for production retry refactor
pub(crate) enum AttemptOutcome {
    /// Request succeeded with a successful status code.
    Success,
    /// Request completed but returned an error status code.
    ErrorStatus(StatusCode),
    /// Request failed due to a transport error (timeout, connection, etc.).
    TransportError {
        is_timeout: bool,
        is_connect: bool,
        is_request: bool,
    },
}

/// Decision from retry evaluation - what the caller should do next.
#[allow(dead_code)] // Used in tests; ready for production retry refactor
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RetryDecision {
    /// Request succeeded, return the response.
    Succeed,
    /// Should retry after the specified delay.
    Retry { delay: Duration },
    /// Should not retry, fail with the current result.
    Fail,
}

/// Pure function to decide whether to retry an HTTP request.
///
/// This function contains no I/O - it takes the outcome of an attempt and
/// returns a decision about what to do next. The caller is responsible for
/// executing the decision (sleeping, making the next request, etc.).
///
/// # Arguments
/// * `outcome` - The result of the current attempt
/// * `attempt` - Current attempt number (1-indexed)
/// * `max_attempts` - Maximum number of attempts allowed
/// * `retry` - Retry configuration (backoff, retry policies)
/// * `method` - HTTP method (affects whether POST requests are retried)
#[allow(dead_code)] // Used in tests; ready for production retry refactor
pub(crate) fn decide_retry(
    outcome: &AttemptOutcome,
    attempt: u32,
    max_attempts: u32,
    retry: &RetryConfig,
    method: &Method,
) -> RetryDecision {
    match outcome {
        AttemptOutcome::Success => RetryDecision::Succeed,
        AttemptOutcome::ErrorStatus(status) => {
            let should_retry = retry.should_retry_status(method, *status);
            if should_retry && attempt < max_attempts {
                RetryDecision::Retry {
                    delay: retry.backoff_delay(attempt),
                }
            } else {
                RetryDecision::Fail
            }
        }
        AttemptOutcome::TransportError {
            is_timeout,
            is_connect,
            is_request,
        } => {
            // Determine if we should retry based on error type
            let should_retry = if *is_timeout || *is_connect || *is_request {
                // Check if method allows retry (POST may be disabled)
                if method == Method::POST {
                    retry.retry_post
                } else {
                    true
                }
            } else {
                false
            };

            if should_retry && attempt < max_attempts {
                RetryDecision::Retry {
                    delay: retry.backoff_delay(attempt),
                }
            } else {
                RetryDecision::Fail
            }
        }
    }
}

/// Classify a reqwest error into an AttemptOutcome for retry decisions.
///
/// This is a pure function that converts error characteristics into our
/// domain model for retry logic.
#[allow(dead_code)] // Ready for production retry refactor
pub(crate) fn classify_error_for_retry(err: &reqwest::Error) -> AttemptOutcome {
    AttemptOutcome::TransportError {
        is_timeout: err.is_timeout(),
        is_connect: err.is_connect(),
        is_request: err.is_request(),
    }
}

/// Raw NDJSON event before parsing.
#[cfg(feature = "streaming")]
#[derive(Clone)]
pub(crate) struct RawEvent {
    pub(crate) data: String,
}

/// Consumes complete NDJSON lines from a buffer.
///
/// Returns a vector of raw events and the remaining incomplete buffer.
#[cfg(feature = "streaming")]
pub(crate) fn consume_ndjson_buffer(buffer: &str) -> (Vec<RawEvent>, String) {
    let mut events = Vec::new();
    let mut remainder = buffer.to_string();
    loop {
        if let Some(idx) = remainder.find('\n') {
            let line = remainder[..idx].to_string();
            remainder = remainder[idx + 1..].to_string();
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            events.push(RawEvent {
                data: trimmed.to_string(),
            });
            continue;
        }
        break;
    }
    (events, remainder)
}

/// Truncate raw data for error messages (avoid huge payloads in logs).
#[cfg(feature = "streaming")]
fn truncate_for_error(data: &str, max_len: usize) -> String {
    if data.len() <= max_len {
        data.to_string()
    } else {
        format!("{}... ({} bytes total)", &data[..max_len], data.len())
    }
}

/// Maps a raw NDJSON event to a StreamEvent.
///
/// Returns:
/// - `Ok(Some(event))` - Successfully parsed event
/// - `Ok(None)` - Expected skip (keepalive events)
/// - `Err(...)` - Parse/protocol error that should be surfaced
///
/// Unified NDJSON format:
/// - `{"type":"start","request_id":"...","model":"..."}`
/// - `{"type":"update","payload":{"content":"..."},"complete_fields":[]}`
/// - `{"type":"completion","payload":{"content":"..."},"usage":{...},"stop_reason":"..."}`
/// - `{"type":"error","code":"...","message":"...","status":...}`
#[cfg(feature = "streaming")]
pub(crate) fn map_event(raw: RawEvent, request_id: Option<String>) -> Result<Option<StreamEvent>> {
    let payload: serde_json::Value =
        serde_json::from_str(&raw.data).map_err(|e| Error::StreamProtocol {
            message: format!("failed to parse NDJSON: {}", e),
            raw_data: Some(truncate_for_error(&raw.data, 200)),
        })?;

    let obj = payload.as_object().ok_or_else(|| Error::StreamProtocol {
        message: "NDJSON event is not an object".to_string(),
        raw_data: Some(truncate_for_error(&raw.data, 200)),
    })?;

    let record_type =
        obj.get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::StreamProtocol {
                message: "NDJSON event missing 'type' field".to_string(),
                raw_data: Some(truncate_for_error(&raw.data, 200)),
            })?;

    // Filter keepalive events (expected, no error needed)
    if record_type == "keepalive" {
        return Ok(None);
    }

    let kind = StreamEventKind::from_event_name(record_type);

    // Keep a reference to raw data for error messages before moving into event
    let raw_data_for_errors = raw.data.clone();

    let mut event = StreamEvent {
        kind,
        event: record_type.to_string(),
        data: Some(payload.clone()),
        text_delta: None,
        tool_call_delta: None,
        tool_calls: None,
        response_id: None,
        model: None,
        stop_reason: None,
        usage: None,
        request_id,
        raw: raw.data,
    };

    // Extract common fields from top level
    event.response_id = obj
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    event.model = obj
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(Model::from);

    event.stop_reason = obj
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(StopReason::from);

    // Extract usage
    if let Some(usage_value) = obj.get("usage") {
        if let Ok(usage) = serde_json::from_value::<Usage>(usage_value.clone()) {
            event.usage = Some(usage);
        }
    }

    // Extract content from payload for update/completion events
    if let Some(payload_obj) = obj.get("payload").and_then(|v| v.as_object()) {
        // For text content, extract from payload.content
        if let Some(content) = payload_obj.get("content").and_then(|v| v.as_str()) {
            event.text_delta = Some(content.to_string());
        }
    }

    // Parse tool call delta - fail fast on malformed data
    if let Some(delta) = obj.get("tool_call_delta") {
        event.tool_call_delta = Some(
            serde_json::from_value::<ToolCallDelta>(delta.clone()).map_err(|e| {
                Error::StreamProtocol {
                    message: format!("failed to parse tool_call_delta: {}", e),
                    raw_data: Some(truncate_for_error(&raw_data_for_errors, 200)),
                }
            })?,
        );
    }

    // Parse tool calls - fail fast on malformed data
    if let Some(tool_calls_value) = obj.get("tool_calls") {
        event.tool_calls = Some(
            serde_json::from_value::<Vec<ToolCall>>(tool_calls_value.clone()).map_err(|e| {
                Error::StreamProtocol {
                    message: format!("failed to parse tool_calls: {}", e),
                    raw_data: Some(truncate_for_error(&raw_data_for_errors, 200)),
                }
            })?,
        );
    }
    if let Some(tool_call_value) = obj.get("tool_call") {
        event.tool_calls = Some(vec![serde_json::from_value::<ToolCall>(
            tool_call_value.clone(),
        )
        .map_err(|e| Error::StreamProtocol {
            message: format!("failed to parse tool_call: {}", e),
            raw_data: Some(truncate_for_error(&raw_data_for_errors, 200)),
        })?]);
    }

    Ok(Some(event))
}

#[cfg(all(test, feature = "streaming"))]
mod tests {
    use super::*;

    #[test]
    fn consumes_ndjson_lines() {
        let data = r#"{"type":"start","request_id":"req-1","model":"gpt-4"}
{"type":"update","payload":{"content":"Hello"}}
{"type":"completion","payload":{"content":"Hello world"},"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}
"#;
        let (events, remainder) = consume_ndjson_buffer(data);
        assert_eq!(remainder, "");
        assert_eq!(events.len(), 3);

        let start = map_event(events[0].clone(), None).unwrap().unwrap();
        assert_eq!(start.kind, StreamEventKind::MessageStart);
        assert_eq!(start.response_id, Some("req-1".to_string()));
        assert_eq!(start.model.as_ref().map(|m| m.as_str()), Some("gpt-4"));

        let update = map_event(events[1].clone(), None).unwrap().unwrap();
        assert_eq!(update.kind, StreamEventKind::MessageDelta);
        assert_eq!(update.text_delta, Some("Hello".to_string()));

        let completion = map_event(events[2].clone(), None).unwrap().unwrap();
        assert_eq!(completion.kind, StreamEventKind::MessageStop);
        assert_eq!(completion.text_delta, Some("Hello world".to_string()));
        assert_eq!(completion.stop_reason, Some(StopReason::EndTurn));
        assert!(completion.usage.is_some());
        assert_eq!(completion.usage.as_ref().unwrap().input_tokens, 1);
    }

    #[test]
    fn filters_keepalive_events() {
        let data = r#"{"type":"keepalive"}
"#;
        let (events, _) = consume_ndjson_buffer(data);
        assert_eq!(events.len(), 1);
        let evt = map_event(events[0].clone(), None).unwrap();
        assert!(evt.is_none(), "keepalive events should be filtered out");
    }

    #[test]
    fn handles_incomplete_buffer() {
        let data = r#"{"type":"start","request_id":"req-1"}"#;
        let (events, remainder) = consume_ndjson_buffer(data);
        assert_eq!(events.len(), 0);
        assert_eq!(remainder, data);
    }

    #[test]
    fn parses_tool_use_events() {
        let data = r#"{"type":"tool_use_start","tool_call_delta":{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather"}}}
{"type":"tool_use_delta","tool_call_delta":{"index":0,"function":{"arguments":"{\"location\":"}}}
{"type":"tool_use_stop","tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"NYC\"}"}}]}
"#;
        let (events, remainder) = consume_ndjson_buffer(data);
        assert_eq!(remainder, "");
        assert_eq!(events.len(), 3);

        let start = map_event(events[0].clone(), None).unwrap().unwrap();
        assert_eq!(start.kind, StreamEventKind::ToolUseStart);
        assert!(start.tool_call_delta.is_some());
        let delta = start.tool_call_delta.unwrap();
        assert_eq!(delta.index, 0);
        assert_eq!(delta.id, Some("call_1".to_string()));

        let delta_event = map_event(events[1].clone(), None).unwrap().unwrap();
        assert_eq!(delta_event.kind, StreamEventKind::ToolUseDelta);
        assert!(delta_event.tool_call_delta.is_some());

        let stop = map_event(events[2].clone(), None).unwrap().unwrap();
        assert_eq!(stop.kind, StreamEventKind::ToolUseStop);
        assert!(stop.tool_calls.is_some());
        let tool_calls = stop.tool_calls.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
    }

    #[test]
    fn parses_single_tool_call_field() {
        let data = r#"{"type":"tool_use_stop","tool_call":{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}}
"#;
        let (events, _) = consume_ndjson_buffer(data);
        assert_eq!(events.len(), 1);

        let stop = map_event(events[0].clone(), None).unwrap().unwrap();
        assert_eq!(stop.kind, StreamEventKind::ToolUseStop);
        assert!(stop.tool_calls.is_some());
        let tool_calls = stop.tool_calls.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
    }

    #[test]
    fn returns_error_on_invalid_json() {
        let raw = RawEvent {
            data: "not valid json".to_string(),
        };
        let result = map_event(raw, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, Error::StreamProtocol { .. }));
    }

    #[test]
    fn returns_error_on_missing_type_field() {
        let raw = RawEvent {
            data: r#"{"foo":"bar"}"#.to_string(),
        };
        let result = map_event(raw, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, Error::StreamProtocol { .. }));
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    fn test_retry_config() -> RetryConfig {
        RetryConfig {
            max_attempts: 3,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            retry_post: true,
        }
    }

    #[test]
    fn decide_retry_succeeds_on_success() {
        let config = test_retry_config();
        let decision = decide_retry(&AttemptOutcome::Success, 1, 3, &config, &Method::GET);
        assert_eq!(decision, RetryDecision::Succeed);
    }

    #[test]
    fn decide_retry_retries_on_server_error() {
        let config = test_retry_config();
        let decision = decide_retry(
            &AttemptOutcome::ErrorStatus(StatusCode::INTERNAL_SERVER_ERROR),
            1,
            3,
            &config,
            &Method::GET,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn decide_retry_fails_on_last_attempt() {
        let config = test_retry_config();
        let decision = decide_retry(
            &AttemptOutcome::ErrorStatus(StatusCode::INTERNAL_SERVER_ERROR),
            3,
            3,
            &config,
            &Method::GET,
        );
        assert_eq!(decision, RetryDecision::Fail);
    }

    #[test]
    fn decide_retry_fails_on_client_error() {
        let config = test_retry_config();
        let decision = decide_retry(
            &AttemptOutcome::ErrorStatus(StatusCode::BAD_REQUEST),
            1,
            3,
            &config,
            &Method::GET,
        );
        assert_eq!(decision, RetryDecision::Fail);
    }

    #[test]
    fn decide_retry_retries_on_timeout() {
        let config = test_retry_config();
        let decision = decide_retry(
            &AttemptOutcome::TransportError {
                is_timeout: true,
                is_connect: false,
                is_request: false,
            },
            1,
            3,
            &config,
            &Method::GET,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn decide_retry_respects_retry_post_config() {
        let mut config = test_retry_config();
        config.retry_post = false;

        let decision = decide_retry(
            &AttemptOutcome::TransportError {
                is_timeout: true,
                is_connect: false,
                is_request: false,
            },
            1,
            3,
            &config,
            &Method::POST,
        );
        assert_eq!(decision, RetryDecision::Fail);

        // GET should still retry
        let decision = decide_retry(
            &AttemptOutcome::TransportError {
                is_timeout: true,
                is_connect: false,
                is_request: false,
            },
            1,
            3,
            &config,
            &Method::GET,
        );
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn decide_retry_returns_backoff_delay() {
        let config = test_retry_config();
        let decision = decide_retry(
            &AttemptOutcome::ErrorStatus(StatusCode::TOO_MANY_REQUESTS),
            2,
            3,
            &config,
            &Method::GET,
        );
        if let RetryDecision::Retry { delay } = decision {
            // Backoff should be between 0.5x and 1.5x of calculated base
            assert!(delay >= Duration::from_millis(50));
            assert!(delay <= Duration::from_secs(1));
        } else {
            panic!("Expected RetryDecision::Retry");
        }
    }
}
