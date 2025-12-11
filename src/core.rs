//! Shared runtime-agnostic logic for async and blocking clients.
//!
//! This module contains data structures and pure functions that are used by both
//! the async client (`client.rs`) and the blocking client (`blocking.rs`).

use reqwest::StatusCode;

use crate::errors::RetryMetadata;
#[cfg(feature = "streaming")]
use crate::errors::{Error, Result};
#[cfg(feature = "streaming")]
use crate::types::{
    Model, StopReason, StreamEvent, StreamEventKind, ToolCall, ToolCallDelta, Usage,
};

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

    // Parse tool call delta
    if let Some(delta) = obj.get("tool_call_delta") {
        match serde_json::from_value::<ToolCallDelta>(delta.clone()) {
            Ok(tool_delta) => event.tool_call_delta = Some(tool_delta),
            Err(e) => {
                #[cfg(feature = "tracing")]
                tracing::warn!(error = %e, "Failed to parse tool_call_delta");
                #[cfg(not(feature = "tracing"))]
                eprintln!("[ModelRelay SDK] Failed to parse tool_call_delta: {}", e);
            }
        }
    }

    // Parse tool calls
    if let Some(tool_calls_value) = obj.get("tool_calls") {
        match serde_json::from_value::<Vec<ToolCall>>(tool_calls_value.clone()) {
            Ok(tool_calls) => event.tool_calls = Some(tool_calls),
            Err(e) => {
                #[cfg(feature = "tracing")]
                tracing::warn!(error = %e, "Failed to parse tool_calls");
                #[cfg(not(feature = "tracing"))]
                eprintln!("[ModelRelay SDK] Failed to parse tool_calls: {}", e);
            }
        }
    }
    if let Some(tool_call_value) = obj.get("tool_call") {
        match serde_json::from_value::<ToolCall>(tool_call_value.clone()) {
            Ok(tool_call) => event.tool_calls = Some(vec![tool_call]),
            Err(e) => {
                #[cfg(feature = "tracing")]
                tracing::warn!(error = %e, "Failed to parse tool_call");
                #[cfg(not(feature = "tracing"))]
                eprintln!("[ModelRelay SDK] Failed to parse tool_call: {}", e);
            }
        }
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
