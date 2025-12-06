use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use futures_core::Stream;
use futures_util::{StreamExt, stream};
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
    event: String,
    data: String,
}

/// Streaming handle over SSE chat events.
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
        let stream = build_stream(
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

    pub(crate) fn new_ndjson(
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

fn build_stream(
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
                    let (events, remainder) = consume_sse_buffer(&buffer, false);
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
                    let (events, _) = consume_sse_buffer(&buffer, true);
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

fn consume_sse_buffer(buffer: &str, flush: bool) -> (Vec<RawEvent>, String) {
    let mut events = Vec::new();
    let mut remainder = buffer.to_string();

    loop {
        if let Some(idx) = remainder.find("\n\n") {
            let (block, rest) = remainder.split_at(idx);
            let rest_owned = rest[2..].to_string();
            if let Some(evt) = parse_event_block(block) {
                events.push(evt);
            }
            remainder = rest_owned;
            continue;
        }
        if flush {
            if let Some(evt) = parse_event_block(&remainder) {
                events.push(evt);
            }
            remainder.clear();
        }
        break;
    }

    (events, remainder)
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
                event: String::new(),
                data: trimmed.to_string(),
            });
            continue;
        }
        break;
    }
    (events, remainder)
}

fn parse_event_block(block: &str) -> Option<RawEvent> {
    let mut event_name = String::new();
    let mut data_lines: Vec<String> = Vec::new();

    for line in block.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = rest.trim().to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim().to_string());
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
    }

    if event_name.is_empty() && data_lines.is_empty() {
        return None;
    }

    Some(RawEvent {
        event: event_name,
        data: data_lines.join("\n"),
    })
}

fn map_event(raw: RawEvent, request_id: Option<String>) -> Option<StreamEvent> {
    let payload = serde_json::from_str::<serde_json::Value>(raw.data.as_str()).ok();
    // NDJSON envelope: { "event": "...", "data": { ... } }
    let inner = payload
        .as_ref()
        .and_then(|v| v.get("data"))
        .cloned()
        .or_else(|| payload.clone())
        .unwrap_or(serde_json::Value::Null);

    let event_hint = inner
        .get("type")
        .or_else(|| inner.get("event"))
        .or_else(|| payload.as_ref().and_then(|v| v.get("event")))
        .and_then(|val| val.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| raw.event.clone());

    let event_name = if event_hint.is_empty() {
        "custom".to_string()
    } else {
        event_hint
    };

    let mut event = StreamEvent {
        kind: StreamEventKind::from_event_name(event_name.as_str()),
        event: if raw.event.is_empty() {
            event_name.clone()
        } else {
            raw.event.clone()
        },
        data: Some(inner.clone()),
        text_delta: None,
        tool_call_delta: None,
        tool_calls: None,
        response_id: None,
        model: None,
        stop_reason: None,
        usage: None,
        request_id,
        raw: inner.to_string(),
    };

    if let Some(obj) = inner.as_object() {
        event.response_id = obj
            .get("response_id")
            .or_else(|| obj.get("responseId"))
            .or_else(|| obj.get("id"))
            .or_else(|| obj.get("message").and_then(|m| m.get("id")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        event.model = obj
            .get("model")
            .or_else(|| obj.get("message").and_then(|m| m.get("model")))
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(Model::from);

        event.stop_reason = obj
            .get("stop_reason")
            .or_else(|| obj.get("stopReason"))
            .and_then(|v| v.as_str())
            .map(StopReason::from);

        if let Some(delta) = obj.get("delta") {
            if let Some(text) = delta.as_str() {
                event.text_delta = Some(text.to_string());
            } else if let Some(delta_obj) = delta.as_object() {
                if let Some(text) = delta_obj
                    .get("text")
                    .or_else(|| delta_obj.get("content"))
                    .and_then(|v| v.as_str())
                {
                    event.text_delta = Some(text.to_string());
                }
            }
        }

        if let Some(usage_value) = obj.get("usage") {
            if let Ok(usage) = serde_json::from_value::<Usage>(usage_value.clone()) {
                event.usage = Some(usage);
            }
        }

        // Parse tool call delta for tool_use_start and tool_use_delta events
        if let Some(delta) = obj.get("tool_call_delta") {
            if let Ok(tool_delta) = serde_json::from_value::<ToolCallDelta>(delta.clone()) {
                event.tool_call_delta = Some(tool_delta);
            }
        }

        // Parse tool calls for tool_use_stop and message_stop events
        if let Some(tool_calls_value) = obj.get("tool_calls") {
            if let Ok(tool_calls) =
                serde_json::from_value::<Vec<ToolCall>>(tool_calls_value.clone())
            {
                event.tool_calls = Some(tool_calls);
            }
        }
        // Single tool_call field for tool_use_stop
        if let Some(tool_call_value) = obj.get("tool_call") {
            if let Ok(tool_call) = serde_json::from_value::<ToolCall>(tool_call_value.clone()) {
                event.tool_calls = Some(vec![tool_call]);
            }
        }
    }

    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_events_from_buffer() {
        let data = "event: message_start\ndata: {\"response_id\":\"r1\"}\n\nevent: message_delta\ndata: {\"delta\":\"hi\"}\n\n";
        let (events, remainder) = consume_sse_buffer(data, false);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "message_start");
        assert_eq!(events[1].event, "message_delta");
        assert_eq!(remainder, "");
    }

    #[test]
    fn flushes_remainder_when_requested() {
        let data = "event: message_stop\ndata: {\"stop_reason\":\"stop_sequence\"}";
        let (events, remainder) = consume_sse_buffer(data, true);
        assert_eq!(remainder, "");
        assert_eq!(events.len(), 1);
        let evt = map_event(events[0].clone(), None).unwrap();
        assert_eq!(evt.kind, StreamEventKind::MessageStop);
        assert_eq!(evt.stop_reason, Some(StopReason::StopSequence));
    }

    #[test]
    fn consumes_ndjson_lines() {
        let data = "{\"event\":\"message_delta\",\"data\":{\"foo\":\"bar\"}}\n{\"event\":\"message_stop\"}\n";
        let (events, remainder) = consume_ndjson_buffer(data);
        assert_eq!(remainder, "");
        assert_eq!(events.len(), 2);
        let evt = map_event(events[0].clone(), None).unwrap();
        assert_eq!(evt.kind, StreamEventKind::MessageDelta);
        assert_eq!(evt.data.unwrap()["foo"], "bar");
    }
}
