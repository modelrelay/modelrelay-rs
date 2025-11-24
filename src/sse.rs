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
    errors::{Result, TransportError, TransportErrorKind},
    types::{StreamEvent, StreamEventKind, Usage},
};

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
}

impl StreamHandle {
    pub(crate) fn new(response: Response, request_id: Option<String>) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream = build_stream(response, request_id.clone(), cancelled.clone());
        Self {
            request_id,
            stream: Box::pin(stream),
            cancelled,
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
) -> impl Stream<Item = Result<StreamEvent>> + Send {
    let body = response.bytes_stream();
    let state = (
        body,
        String::new(),
        request_id,
        cancelled,
        VecDeque::<StreamEvent>::new(),
    );

    stream::unfold(state, |state| async move {
        let (mut body, mut buffer, request_id, cancelled, mut pending) = state;
        loop {
            if cancelled.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(event) = pending.pop_front() {
                return Some((Ok(event), (body, buffer, request_id, cancelled, pending)));
            }
            match body.next().await {
                Some(Ok(chunk)) => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    let (events, remainder) = consume_sse_buffer(&buffer, false);
                    buffer = remainder;
                    for raw in events {
                        if let Some(evt) = map_event(raw, request_id.clone()) {
                            pending.push_back(evt);
                        }
                    }
                    if let Some(event) = pending.pop_front() {
                        return Some((Ok(event), (body, buffer, request_id, cancelled, pending)));
                    }
                }
                Some(Err(err)) => {
                    return Some((
                        Err(TransportError {
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
                        }
                        .into()),
                        (body, buffer, request_id, cancelled, pending),
                    ));
                }
                None => {
                    let (events, _) = consume_sse_buffer(&buffer, true);
                    buffer.clear();
                    for raw in events {
                        if let Some(evt) = map_event(raw, request_id.clone()) {
                            pending.push_back(evt);
                        }
                    }
                    if let Some(event) = pending.pop_front() {
                        return Some((Ok(event), (body, buffer, request_id, cancelled, pending)));
                    }
                    return None;
                }
            }
        }
    })
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
    let event_hint = payload
        .as_ref()
        .and_then(|v| {
            v.get("type")
                .or_else(|| v.get("event"))
                .and_then(|val| val.as_str())
        })
        .and_then(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
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
        data: payload.clone(),
        text_delta: None,
        response_id: None,
        model: None,
        stop_reason: None,
        usage: None,
        request_id,
        raw: raw.data.clone(),
    };

    if let Some(value) = payload {
        if let Some(obj) = value.as_object() {
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
                .map(|s| s.to_string());

            event.stop_reason = obj
                .get("stop_reason")
                .or_else(|| obj.get("stopReason"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

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
        assert_eq!(evt.stop_reason.as_deref(), Some("stop_sequence"));
    }
}
