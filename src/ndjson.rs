use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Instant,
};

use futures_core::Stream;
use futures_util::{stream, StreamExt};
use reqwest::Response as HttpResponse;

use crate::{
    core::{consume_ndjson_buffer, map_event},
    errors::{
        Error, Result, StreamTimeoutError, StreamTimeoutKind, TransportError, TransportErrorKind,
    },
    http::StreamTimeouts,
    telemetry::StreamTelemetry,
    tools::ToolCallAccumulator,
    types::{
        ContentPart, Model, OutputItem, Response, StopReason, StreamEvent, StreamEventKind, Usage,
    },
};

const MAX_PENDING_EVENTS: usize = 512;

/// Streaming handle over NDJSON response events.
pub struct StreamHandle {
    request_id: Option<String>,
    stream: Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>,
    cancelled: Arc<AtomicBool>,
    telemetry: Option<StreamTelemetry>,
}

impl StreamHandle {
    pub(crate) fn new(
        response: HttpResponse,
        request_id: Option<String>,
        telemetry: Option<StreamTelemetry>,
        timeouts: StreamTimeouts,
        started_at: Instant,
    ) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream = build_ndjson_stream(
            response,
            request_id.clone(),
            cancelled.clone(),
            telemetry.clone(),
            timeouts,
            started_at,
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

    /// Collect the streaming response into a full `Response` (non-streaming aggregate).
    pub async fn collect(mut self) -> Result<Response> {
        use futures_util::StreamExt;

        let mut content = String::new();
        let mut response_id: Option<String> = None;
        let mut model: Option<Model> = None;
        let mut usage: Option<Usage> = None;
        let mut stop_reason: Option<StopReason> = None;
        let mut tool_calls = None;
        let request_id = self.request_id.clone();
        let mut tool_acc = ToolCallAccumulator::default();

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
                StreamEventKind::ToolUseStart | StreamEventKind::ToolUseDelta => {
                    if let Some(delta) = evt.tool_call_delta {
                        tool_acc.process_delta(&delta);
                    }
                }
                StreamEventKind::ToolUseStop => {
                    if evt.tool_calls.is_some() {
                        tool_calls = evt.tool_calls;
                    }
                }
                StreamEventKind::MessageStop => {
                    stop_reason = evt.stop_reason.or(stop_reason);
                    usage = evt.usage.or(usage);
                    response_id = evt.response_id.or(response_id);
                    model = evt.model.or(model);
                    if evt.tool_calls.is_some() {
                        tool_calls = evt.tool_calls;
                    }
                    break;
                }
                _ => {}
            }
        }

        let tool_calls = tool_calls.or_else(|| {
            let calls = tool_acc.get_tool_calls();
            if calls.is_empty() {
                None
            } else {
                Some(calls)
            }
        });

        let output = vec![OutputItem::Message {
            role: crate::types::MessageRole::Assistant,
            content: vec![ContentPart::text(content)],
            tool_calls,
        }];

        Ok(Response {
            id: response_id
                .or_else(|| request_id.clone())
                .unwrap_or_else(|| "stream".to_string()),
            stop_reason,
            model: model.unwrap_or_else(|| Model::new(String::new())),
            output,
            usage: usage.unwrap_or_default(),
            request_id,
            provider: None,
            citations: None,
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
    response: HttpResponse,
    request_id: Option<String>,
    cancelled: Arc<AtomicBool>,
    telemetry: Option<StreamTelemetry>,
    timeouts: StreamTimeouts,
    started_at: Instant,
) -> impl Stream<Item = Result<StreamEvent>> + Send {
    let body = response.bytes_stream();
    let last_activity = started_at;
    let state = (
        body,
        String::new(),
        request_id,
        cancelled,
        VecDeque::<StreamEvent>::new(),
        telemetry,
        timeouts,
        started_at,
        last_activity,
        false, // ttft_satisfied
    );

    stream::unfold(state, |state| async move {
        let (
            mut body,
            mut buffer,
            request_id,
            cancelled,
            mut pending,
            telemetry,
            timeouts,
            started_at,
            mut last_activity,
            mut ttft_satisfied,
        ) = state;
        loop {
            if cancelled.load(Ordering::SeqCst) {
                if let Some(t) = telemetry.as_ref() {
                    t.on_closed();
                }
                return None;
            }

            if let Some(total) = timeouts.total {
                if started_at.elapsed() >= total {
                    let err = Error::StreamTimeout(StreamTimeoutError {
                        kind: StreamTimeoutKind::Total,
                        timeout: total,
                    });
                    cancelled.store(true, Ordering::SeqCst);
                    if let Some(t) = telemetry.as_ref() {
                        t.on_error(&err);
                    }
                    return Some((
                        Err(err),
                        (
                            body,
                            buffer,
                            request_id,
                            cancelled,
                            pending,
                            telemetry,
                            timeouts,
                            started_at,
                            last_activity,
                            ttft_satisfied,
                        ),
                    ));
                }
            }
            if !ttft_satisfied {
                if let Some(ttft) = timeouts.ttft {
                    if started_at.elapsed() >= ttft {
                        let err = Error::StreamTimeout(StreamTimeoutError {
                            kind: StreamTimeoutKind::Ttft,
                            timeout: ttft,
                        });
                        cancelled.store(true, Ordering::SeqCst);
                        if let Some(t) = telemetry.as_ref() {
                            t.on_error(&err);
                        }
                        return Some((
                            Err(err),
                            (
                                body,
                                buffer,
                                request_id,
                                cancelled,
                                pending,
                                telemetry,
                                timeouts,
                                started_at,
                                last_activity,
                                ttft_satisfied,
                            ),
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
                    (
                        body,
                        buffer,
                        request_id,
                        cancelled,
                        pending,
                        telemetry,
                        timeouts,
                        started_at,
                        last_activity,
                        ttft_satisfied,
                    ),
                ));
            }

            if let Some(idle) = timeouts.idle {
                if last_activity.elapsed() >= idle {
                    let err = Error::StreamTimeout(StreamTimeoutError {
                        kind: StreamTimeoutKind::Idle,
                        timeout: idle,
                    });
                    cancelled.store(true, Ordering::SeqCst);
                    if let Some(t) = telemetry.as_ref() {
                        t.on_error(&err);
                    }
                    return Some((
                        Err(err),
                        (
                            body,
                            buffer,
                            request_id,
                            cancelled,
                            pending,
                            telemetry,
                            timeouts,
                            started_at,
                            last_activity,
                            ttft_satisfied,
                        ),
                    ));
                }
            }

            let next_deadline =
                stream_next_deadline(timeouts, started_at, last_activity, ttft_satisfied);
            let next_item = if let Some((_, remaining, _)) = next_deadline {
                match tokio::time::timeout(remaining, body.next()).await {
                    Ok(v) => v,
                    Err(_) => {
                        let (kind, _, configured) = next_deadline.expect("deadline must exist");
                        let err = Error::StreamTimeout(StreamTimeoutError {
                            kind,
                            timeout: configured,
                        });
                        cancelled.store(true, Ordering::SeqCst);
                        if let Some(t) = telemetry.as_ref() {
                            t.on_error(&err);
                        }
                        return Some((
                            Err(err),
                            (
                                body,
                                buffer,
                                request_id,
                                cancelled,
                                pending,
                                telemetry,
                                timeouts,
                                started_at,
                                last_activity,
                                ttft_satisfied,
                            ),
                        ));
                    }
                }
            } else {
                body.next().await
            };

            match next_item {
                Some(Ok(chunk)) => {
                    last_activity = Instant::now();
                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    let (events, remainder) = consume_ndjson_buffer(&buffer);
                    buffer = remainder;
                    for raw in events {
                        match map_event(raw, request_id.clone()) {
                            Ok(Some(evt)) => {
                                if !ttft_satisfied && event_counts_for_ttft(&evt) {
                                    ttft_satisfied = true;
                                }
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
                                        (
                                            body,
                                            buffer,
                                            request_id,
                                            cancelled,
                                            pending,
                                            telemetry,
                                            timeouts,
                                            started_at,
                                            last_activity,
                                            ttft_satisfied,
                                        ),
                                    ));
                                }
                            }
                            Ok(None) => {} // keepalive, skip
                            Err(err) => {
                                if let Some(t) = telemetry.as_ref() {
                                    t.on_error(&err);
                                }
                                return Some((
                                    Err(err),
                                    (
                                        body,
                                        buffer,
                                        request_id,
                                        cancelled,
                                        pending,
                                        telemetry,
                                        timeouts,
                                        started_at,
                                        last_activity,
                                        ttft_satisfied,
                                    ),
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
                            (
                                body,
                                buffer,
                                request_id,
                                cancelled,
                                pending,
                                telemetry,
                                timeouts,
                                started_at,
                                last_activity,
                                ttft_satisfied,
                            ),
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
                        (
                            body,
                            buffer,
                            request_id,
                            cancelled,
                            pending,
                            telemetry,
                            timeouts,
                            started_at,
                            last_activity,
                            ttft_satisfied,
                        ),
                    ));
                }
                None => {
                    let (events, _) = consume_ndjson_buffer(&buffer);
                    buffer.clear();
                    for raw in events {
                        match map_event(raw, request_id.clone()) {
                            Ok(Some(evt)) => {
                                if !ttft_satisfied && event_counts_for_ttft(&evt) {
                                    ttft_satisfied = true;
                                }
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
                                        (
                                            body,
                                            buffer,
                                            request_id,
                                            cancelled,
                                            pending,
                                            telemetry,
                                            timeouts,
                                            started_at,
                                            last_activity,
                                            ttft_satisfied,
                                        ),
                                    ));
                                }
                            }
                            Ok(None) => {} // keepalive, skip
                            Err(err) => {
                                if let Some(t) = telemetry.as_ref() {
                                    t.on_error(&err);
                                }
                                return Some((
                                    Err(err),
                                    (
                                        body,
                                        buffer,
                                        request_id,
                                        cancelled,
                                        pending,
                                        telemetry,
                                        timeouts,
                                        started_at,
                                        last_activity,
                                        ttft_satisfied,
                                    ),
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
                            (
                                body,
                                buffer,
                                request_id,
                                cancelled,
                                pending,
                                telemetry,
                                timeouts,
                                started_at,
                                last_activity,
                                ttft_satisfied,
                            ),
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

fn event_counts_for_ttft(event: &StreamEvent) -> bool {
    if let Some(text) = &event.text_delta {
        if !text.is_empty() {
            return true;
        }
    }
    if event.tool_call_delta.is_some() {
        return true;
    }
    if event
        .tool_calls
        .as_ref()
        .map(|c| !c.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    event.event == "error"
}

fn stream_next_deadline(
    timeouts: StreamTimeouts,
    started_at: Instant,
    last_activity: Instant,
    ttft_satisfied: bool,
) -> Option<(StreamTimeoutKind, std::time::Duration, std::time::Duration)> {
    let now_total = started_at.elapsed();
    let mut best: Option<(StreamTimeoutKind, std::time::Duration, std::time::Duration)> = None;

    if let Some(total) = timeouts.total {
        if total > now_total {
            let rem = total - now_total;
            best = Some((StreamTimeoutKind::Total, rem, total));
        } else {
            return Some((
                StreamTimeoutKind::Total,
                std::time::Duration::from_millis(0),
                total,
            ));
        }
    }
    if let Some(idle) = timeouts.idle {
        let elapsed = last_activity.elapsed();
        if idle > elapsed {
            let rem = idle - elapsed;
            best = match best {
                Some((k, brem, bcfg)) if brem <= rem => Some((k, brem, bcfg)),
                _ => Some((StreamTimeoutKind::Idle, rem, idle)),
            };
        } else {
            return Some((
                StreamTimeoutKind::Idle,
                std::time::Duration::from_millis(0),
                idle,
            ));
        }
    }
    if !ttft_satisfied {
        if let Some(ttft) = timeouts.ttft {
            if ttft > now_total {
                let rem = ttft - now_total;
                best = match best {
                    Some((k, brem, bcfg)) if brem <= rem => Some((k, brem, bcfg)),
                    _ => Some((StreamTimeoutKind::Ttft, rem, ttft)),
                };
            } else {
                return Some((
                    StreamTimeoutKind::Ttft,
                    std::time::Duration::from_millis(0),
                    ttft,
                ));
            }
        }
    }

    best
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

// NDJSON parsing functions are now in core.rs
