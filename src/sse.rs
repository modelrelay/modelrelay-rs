use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use futures_core::Stream;
use futures_util::{stream, StreamExt};
use reqwest::Response;

use crate::{
    errors::{Error, Result, TransportError, TransportErrorKind},
    telemetry::StreamTelemetry,
    types::{
        Model, ProxyResponse, StopReason, StreamEvent, StreamEventKind, ToolCall, ToolCallDelta,
        Usage,
    },
};

const MAX_PENDING_EVENTS: usize = 512;

#[derive(Clone)]
struct RawEvent {
    data: String,
}

/// Streaming handle over NDJSON chat events.
pub struct StreamHandle {
    request_id: Option<String>,
    stream: Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>,
    cancelled: Arc<AtomicBool>,
    telemetry: Option<StreamTelemetry>,
}

impl StreamHandle {
    pub(crate) fn new(
        response: Response,
        request_id: Option<String>,
        telemetry: Option<StreamTelemetry>,
    ) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream = build_ndjson_stream(
            response,
            request_id.clone(),
            cancelled.clone(),
            telemetry.clone(),
        );
        Self {
            request_id,
            stream: Box::pin(stream),
            cancelled,
            telemetry,
        }
    }

    /// Build a stream handle from a sequence of events (useful for tests/mocks).
    pub fn from_events(events: impl IntoIterator<Item = StreamEvent>) -> Self {
        Self::from_events_with_request_id(events, None)
    }

    /// Build a stream handle from events and an explicit request id.
    pub fn from_events_with_request_id(
        events: impl IntoIterator<Item = StreamEvent>,
        request_id: Option<String>,
    ) -> Self {
        let collected: Vec<StreamEvent> = events.into_iter().collect();
        let req_id = request_id.or_else(|| collected.iter().find_map(|evt| evt.request_id.clone()));
        let stream = stream::iter(collected.into_iter().map(Ok));
        Self::from_stream(stream, req_id, None)
    }

    pub(crate) fn from_stream<S>(
        stream: S,
        request_id: Option<String>,
        telemetry: Option<StreamTelemetry>,
    ) -> Self
    where
        S: Stream<Item = Result<StreamEvent>> + Send + 'static,
    {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream = build_custom_stream(stream, cancelled.clone(), telemetry.clone());
        Self {
            request_id,
            stream: Box::pin(stream),
            cancelled,
            telemetry,
        }
    }

    /// Request identifier returned by the server (if any).
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// Cancel the in-flight streaming request.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Collect the streaming response into a full `ProxyResponse` (non-streaming aggregate).
    pub async fn collect(mut self) -> Result<ProxyResponse> {
        use futures_util::StreamExt;

        let mut content = String::new();
        let mut response_id: Option<String> = None;
        let mut model: Option<Model> = None;
        let mut usage: Option<Usage> = None;
        let mut stop_reason: Option<StopReason> = None;
        let request_id = self.request_id.clone();

        while let Some(item) = self.next().await {
            let evt = item?;
            match evt.kind {
                StreamEventKind::MessageDelta => {
                    if let Some(delta) = evt.text_delta {
                        content.push_str(&delta);
                    }
                    if response_id.is_none() {
                        response_id = evt.response_id.clone();
                    }
                    if model.is_none() {
                        model = evt.model.clone();
                    }
                }
                StreamEventKind::MessageStart => {
                    if response_id.is_none() {
                        response_id = evt.response_id.clone();
                    }
                    if model.is_none() {
                        model = evt.model.clone();
                    }
                }
                StreamEventKind::MessageStop => {
                    stop_reason = evt.stop_reason.or(stop_reason);
                    usage = evt.usage.or(usage);
                    response_id = evt.response_id.or(response_id);
                    model = evt.model.or(model);
                    break;
                }
                _ => {}
            }
        }

        Ok(ProxyResponse {
            id: response_id
                .or_else(|| request_id.clone())
                .unwrap_or_else(|| "stream".to_string()),
            content: vec![content],
            stop_reason,
            model: model.unwrap_or_else(|| Model::new(String::new())),
            usage: usage.unwrap_or_default(),
            request_id,
            tool_calls: None,
        })
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.cancel();
        if let Some(t) = self.telemetry.take() {
            t.on_closed();
        }
    }
}

impl Stream for StreamHandle {
    type Item = Result<StreamEvent>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = unsafe { self.map_unchecked_mut(|s| &mut s.stream) };
        stream.poll_next(cx)
    }
}

fn build_ndjson_stream(
    response: Response,
    request_id: Option<String>,
    cancelled: Arc<AtomicBool>,
    telemetry: Option<StreamTelemetry>,
) -> impl Stream<Item = Result<StreamEvent>> + Send {
    let body = response.bytes_stream();
    let state = (
        body,
        String::new(),
        request_id,
        cancelled,
        VecDeque::<StreamEvent>::new(),
        telemetry,
    );

    stream::unfold(state, |state| async move {
        let (mut body, mut buffer, request_id, cancelled, mut pending, telemetry) = state;
        loop {
            if cancelled.load(Ordering::SeqCst) {
                if let Some(t) = telemetry.as_ref() {
                    t.on_closed();
                }
                return None;
            }
            if let Some(event) = pending.pop_front() {
                if let Some(t) = telemetry.as_ref() {
                    t.on_event(&event);
                }
                return Some((
                    Ok(event),
                    (body, buffer, request_id, cancelled, pending, telemetry),
                ));
            }
            match body.next().await {
                Some(Ok(chunk)) => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    let (events, remainder) = consume_ndjson_buffer(&buffer);
                    buffer = remainder;
                    for raw in events {
                        if let Some(evt) = map_event(raw, request_id.clone()) {
                            pending.push_back(evt);
                            if pending.len() > MAX_PENDING_EVENTS {
                                let err = Error::StreamBackpressure {
                                    dropped: pending.len(),
                                };
                                if let Some(t) = telemetry.as_ref() {
                                    t.on_error(&err);
                                }
                                return Some((
                                    Err(err),
                                    (body, buffer, request_id, cancelled, pending, telemetry),
                                ));
                            }
                        }
                    }
                    if let Some(event) = pending.pop_front() {
                        if let Some(t) = telemetry.as_ref() {
                            t.on_event(&event);
                        }
                        return Some((
                            Ok(event),
                            (body, buffer, request_id, cancelled, pending, telemetry),
                        ));
                    }
                }
                Some(Err(err)) => {
                    let error = Error::Transport(TransportError {
                        kind: if err.is_timeout() {
                            TransportErrorKind::Timeout
                        } else if err.is_connect() {
                            TransportErrorKind::Connect
                        } else if err.is_request() {
                            TransportErrorKind::Request
                        } else {
                            TransportErrorKind::Other
                        },
                        message: err.to_string(),
                        source: Some(err),
                        retries: None,
                    });
                    if let Some(t) = telemetry.as_ref() {
                        t.on_error(&error);
                    }
                    return Some((
                        Err(error),
                        (body, buffer, request_id, cancelled, pending, telemetry),
                    ));
                }
                None => {
                    let (events, _) = consume_ndjson_buffer(&buffer);
                    buffer.clear();
                    for raw in events {
                        if let Some(evt) = map_event(raw, request_id.clone()) {
                            pending.push_back(evt);
                            if pending.len() > MAX_PENDING_EVENTS {
                                let err = Error::StreamBackpressure {
                                    dropped: pending.len(),
                                };
                                if let Some(t) = telemetry.as_ref() {
                                    t.on_error(&err);
                                }
                                return Some((
                                    Err(err),
                                    (body, buffer, request_id, cancelled, pending, telemetry),
                                ));
                            }
                        }
                    }
                    if let Some(event) = pending.pop_front() {
                        if let Some(t) = telemetry.as_ref() {
                            t.on_event(&event);
                        }
                        return Some((
                            Ok(event),
                            (body, buffer, request_id, cancelled, pending, telemetry),
                        ));
                    }
                    if let Some(t) = telemetry.as_ref() {
                        t.on_closed();
                    }
                    return None;
                }
            }
        }
    })
}

fn build_custom_stream<S>(
    stream: S,
    cancelled: Arc<AtomicBool>,
    telemetry: Option<StreamTelemetry>,
) -> impl Stream<Item = Result<StreamEvent>> + Send
where
    S: Stream<Item = Result<StreamEvent>> + Send + 'static,
{
    stream::unfold(
        (Box::pin(stream), cancelled, telemetry),
        |state| async move {
            let (mut stream, cancelled, telemetry) = state;
            if cancelled.load(Ordering::SeqCst) {
                if let Some(t) = telemetry.as_ref() {
                    t.on_closed();
                }
                return None;
            }
            match stream.next().await {
                Some(item) => {
                    if let Some(t) = telemetry.as_ref() {
                        match &item {
                            Ok(evt) => t.on_event(evt),
                            Err(err) => t.on_error(err),
                        }
                    }
                    Some((item, (stream, cancelled, telemetry)))
                }
                None => {
                    if let Some(t) = telemetry.as_ref() {
                        t.on_closed();
                    }
                    None
                }
            }
        },
    )
}

fn consume_ndjson_buffer(buffer: &str) -> (Vec<RawEvent>, String) {
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

/// Maps a raw NDJSON event to a StreamEvent.
///
/// Unified NDJSON format:
/// - `{"type":"start","request_id":"...","model":"..."}`
/// - `{"type":"update","payload":{"content":"..."},"complete_fields":[]}`
/// - `{"type":"completion","payload":{"content":"..."},"usage":{...},"stop_reason":"..."}`
/// - `{"type":"error","code":"...","message":"...","status":...}`
fn map_event(raw: RawEvent, request_id: Option<String>) -> Option<StreamEvent> {
    let payload: serde_json::Value = match serde_json::from_str(&raw.data) {
        Ok(v) => v,
        Err(e) => {
            #[cfg(feature = "tracing")]
            tracing::warn!(
                error = %e,
                raw_data_len = raw.data.len(),
                "Failed to parse NDJSON event"
            );
            #[cfg(not(feature = "tracing"))]
            eprintln!(
                "[ModelRelay SDK] Failed to parse NDJSON event: {} (data len: {})",
                e,
                raw.data.len()
            );
            return None;
        }
    };

    let obj = match payload.as_object() {
        Some(o) => o,
        None => {
            #[cfg(feature = "tracing")]
            tracing::warn!("NDJSON event is not an object");
            #[cfg(not(feature = "tracing"))]
            eprintln!("[ModelRelay SDK] NDJSON event is not an object");
            return None;
        }
    };

    let record_type = match obj.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => {
            #[cfg(feature = "tracing")]
            tracing::warn!("NDJSON event missing 'type' field");
            #[cfg(not(feature = "tracing"))]
            eprintln!("[ModelRelay SDK] NDJSON event missing 'type' field");
            return None;
        }
    };

    // Filter keepalive events (expected, no warning needed)
    if record_type == "keepalive" {
        return None;
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

    Some(event)
}

#[cfg(test)]
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

        let start = map_event(events[0].clone(), None).unwrap();
        assert_eq!(start.kind, StreamEventKind::MessageStart);
        assert_eq!(start.response_id, Some("req-1".to_string()));
        assert_eq!(start.model.as_ref().map(|m| m.as_str()), Some("gpt-4"));

        let update = map_event(events[1].clone(), None).unwrap();
        assert_eq!(update.kind, StreamEventKind::MessageDelta);
        assert_eq!(update.text_delta, Some("Hello".to_string()));

        let completion = map_event(events[2].clone(), None).unwrap();
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
        let evt = map_event(events[0].clone(), None);
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

        let start = map_event(events[0].clone(), None).unwrap();
        assert_eq!(start.kind, StreamEventKind::ToolUseStart);
        assert!(start.tool_call_delta.is_some());
        let delta = start.tool_call_delta.unwrap();
        assert_eq!(delta.index, 0);
        assert_eq!(delta.id, Some("call_1".to_string()));

        let delta_event = map_event(events[1].clone(), None).unwrap();
        assert_eq!(delta_event.kind, StreamEventKind::ToolUseDelta);
        assert!(delta_event.tool_call_delta.is_some());

        let stop = map_event(events[2].clone(), None).unwrap();
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

        let stop = map_event(events[0].clone(), None).unwrap();
        assert_eq!(stop.kind, StreamEventKind::ToolUseStop);
        assert!(stop.tool_calls.is_some());
        let tool_calls = stop.tool_calls.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
    }
}
