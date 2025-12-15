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
    core::{consume_ndjson_buffer, map_event, RawEvent},
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

/// State for NDJSON stream processing.
///
/// Extracted from tuple to improve readability and eliminate repeated destructuring.
struct NdjsonStreamState<B, E> {
    body: B,
    buffer: String,
    request_id: Option<String>,
    cancelled: Arc<AtomicBool>,
    pending: VecDeque<E>,
    telemetry: Option<StreamTelemetry>,
    timeouts: StreamTimeouts,
    started_at: Instant,
    last_activity: Instant,
    ttft_satisfied: bool,
}

impl<B, E> NdjsonStreamState<B, E> {
    fn new(
        body: B,
        request_id: Option<String>,
        cancelled: Arc<AtomicBool>,
        telemetry: Option<StreamTelemetry>,
        timeouts: StreamTimeouts,
        started_at: Instant,
    ) -> Self {
        Self {
            body,
            buffer: String::new(),
            request_id,
            cancelled,
            pending: VecDeque::new(),
            telemetry,
            timeouts,
            started_at,
            last_activity: started_at,
            ttft_satisfied: false,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn set_cancelled(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn on_telemetry_closed(&self) {
        if let Some(t) = self.telemetry.as_ref() {
            t.on_closed();
        }
    }

    fn on_telemetry_error(&self, err: &Error) {
        if let Some(t) = self.telemetry.as_ref() {
            t.on_error(err);
        }
    }
}

impl<B> NdjsonStreamState<B, StreamEvent> {
    fn on_telemetry_event(&self, evt: &StreamEvent) {
        if let Some(t) = self.telemetry.as_ref() {
            t.on_event(evt);
        }
    }
}

/// Checks if any timeout has been exceeded and returns the appropriate error.
fn check_timeout_exceeded(
    timeouts: StreamTimeouts,
    started_at: Instant,
    last_activity: Instant,
    ttft_satisfied: bool,
) -> Option<StreamTimeoutError> {
    if let Some(total) = timeouts.total {
        if started_at.elapsed() >= total {
            return Some(StreamTimeoutError {
                kind: StreamTimeoutKind::Total,
                timeout: total,
            });
        }
    }
    if !ttft_satisfied {
        if let Some(ttft) = timeouts.ttft {
            if started_at.elapsed() >= ttft {
                return Some(StreamTimeoutError {
                    kind: StreamTimeoutKind::Ttft,
                    timeout: ttft,
                });
            }
        }
    }
    if let Some(idle) = timeouts.idle {
        if last_activity.elapsed() >= idle {
            return Some(StreamTimeoutError {
                kind: StreamTimeoutKind::Idle,
                timeout: idle,
            });
        }
    }
    None
}

/// Classifies a reqwest error into a transport error kind.
pub(crate) fn classify_reqwest_error(err: &reqwest::Error) -> TransportErrorKind {
    if err.is_timeout() {
        TransportErrorKind::Timeout
    } else if err.is_connect() {
        TransportErrorKind::Connect
    } else if err.is_request() {
        TransportErrorKind::Request
    } else {
        TransportErrorKind::Other
    }
}

/// Parses a chunk of bytes into UTF-8 text.
fn parse_utf8_chunk(chunk: &[u8]) -> std::result::Result<String, Error> {
    String::from_utf8(chunk.to_vec()).map_err(|e| Error::StreamProtocol {
        message: format!("invalid UTF-8 in stream: {}", e),
        raw_data: None,
    })
}

/// Result of processing NDJSON events from a buffer.
enum ProcessEventsResult {
    /// Successfully parsed events (possibly empty if only keepalives)
    Ok,
    /// Parse error encountered
    Err(Error),
    /// Backpressure limit exceeded
    Backpressure(usize),
}

/// Processes raw NDJSON events into typed events using a parser function.
fn process_ndjson_events<E, F>(
    events: Vec<RawEvent>,
    pending: &mut VecDeque<E>,
    ttft_satisfied: &mut bool,
    mut parser: F,
    ttft_checker: Option<fn(&E) -> bool>,
) -> ProcessEventsResult
where
    F: FnMut(&RawEvent) -> std::result::Result<Option<E>, Error>,
{
    for raw in events {
        match parser(&raw) {
            Ok(Some(evt)) => {
                if !*ttft_satisfied {
                    if let Some(checker) = ttft_checker {
                        if checker(&evt) {
                            *ttft_satisfied = true;
                        }
                    }
                }
                pending.push_back(evt);
                if pending.len() > MAX_PENDING_EVENTS {
                    return ProcessEventsResult::Backpressure(pending.len());
                }
            }
            Ok(None) => {} // keepalive, skip
            Err(err) => return ProcessEventsResult::Err(err),
        }
    }
    ProcessEventsResult::Ok
}

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

        // Fail fast if stream completed without identifiers or model info
        let id =
            response_id
                .or_else(|| request_id.clone())
                .ok_or_else(|| Error::StreamProtocol {
                    message: "stream completed without response_id or request_id".to_string(),
                    raw_data: None,
                })?;

        let model = model.ok_or_else(|| Error::StreamProtocol {
            message: "stream completed without model information".to_string(),
            raw_data: None,
        })?;

        Ok(Response {
            id,
            stop_reason,
            model,
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
    let state =
        NdjsonStreamState::new(body, request_id, cancelled, telemetry, timeouts, started_at);

    stream::unfold(state, |mut state| async move {
        loop {
            // Check cancellation
            if state.is_cancelled() {
                state.on_telemetry_closed();
                return None;
            }

            // Check timeouts (pure function)
            if let Some(timeout_err) = check_timeout_exceeded(
                state.timeouts,
                state.started_at,
                state.last_activity,
                state.ttft_satisfied,
            ) {
                let err = Error::StreamTimeout(timeout_err);
                state.set_cancelled();
                state.on_telemetry_error(&err);
                return Some((Err(err), state));
            }

            // Return pending events
            if let Some(event) = state.pending.pop_front() {
                state.on_telemetry_event(&event);
                return Some((Ok(event), state));
            }

            // Calculate deadline for next read
            let next_deadline = stream_next_deadline(
                state.timeouts,
                state.started_at,
                state.last_activity,
                state.ttft_satisfied,
            );

            // Read next chunk with optional timeout
            let next_item = if let Some((_, remaining, _)) = next_deadline {
                match tokio::time::timeout(remaining, state.body.next()).await {
                    Ok(v) => v,
                    Err(_) => {
                        let (kind, _, configured) = next_deadline.expect("deadline must exist");
                        let err = Error::StreamTimeout(StreamTimeoutError {
                            kind,
                            timeout: configured,
                        });
                        state.set_cancelled();
                        state.on_telemetry_error(&err);
                        return Some((Err(err), state));
                    }
                }
            } else {
                state.body.next().await
            };

            match next_item {
                Some(Ok(chunk)) => {
                    state.last_activity = Instant::now();

                    // Parse UTF-8
                    let text = match parse_utf8_chunk(&chunk) {
                        Ok(s) => s,
                        Err(err) => {
                            state.on_telemetry_error(&err);
                            return Some((Err(err), state));
                        }
                    };

                    // Parse NDJSON lines
                    state.buffer.push_str(&text);
                    let (events, remainder) = consume_ndjson_buffer(&state.buffer);
                    state.buffer = remainder;

                    // Process events
                    let request_id = state.request_id.clone();
                    let result = process_ndjson_events(
                        events,
                        &mut state.pending,
                        &mut state.ttft_satisfied,
                        |raw| map_event(raw.clone(), request_id.clone()),
                        Some(event_counts_for_ttft),
                    );

                    match result {
                        ProcessEventsResult::Ok => {}
                        ProcessEventsResult::Err(err) => {
                            state.on_telemetry_error(&err);
                            return Some((Err(err), state));
                        }
                        ProcessEventsResult::Backpressure(dropped) => {
                            let err = Error::StreamBackpressure { dropped };
                            state.on_telemetry_error(&err);
                            return Some((Err(err), state));
                        }
                    }

                    // Return first pending event if available
                    if let Some(event) = state.pending.pop_front() {
                        state.on_telemetry_event(&event);
                        return Some((Ok(event), state));
                    }
                }
                Some(Err(err)) => {
                    let error = Error::Transport(TransportError {
                        kind: classify_reqwest_error(&err),
                        message: err.to_string(),
                        source: Some(err),
                        retries: None,
                    });
                    state.on_telemetry_error(&error);
                    return Some((Err(error), state));
                }
                None => {
                    // Stream ended, process remaining buffer
                    let (events, _) = consume_ndjson_buffer(&state.buffer);
                    state.buffer.clear();

                    let request_id = state.request_id.clone();
                    let result = process_ndjson_events(
                        events,
                        &mut state.pending,
                        &mut state.ttft_satisfied,
                        |raw| map_event(raw.clone(), request_id.clone()),
                        Some(event_counts_for_ttft),
                    );

                    match result {
                        ProcessEventsResult::Ok => {}
                        ProcessEventsResult::Err(err) => {
                            state.on_telemetry_error(&err);
                            return Some((Err(err), state));
                        }
                        ProcessEventsResult::Backpressure(dropped) => {
                            let err = Error::StreamBackpressure { dropped };
                            state.on_telemetry_error(&err);
                            return Some((Err(err), state));
                        }
                    }

                    if let Some(event) = state.pending.pop_front() {
                        state.on_telemetry_event(&event);
                        return Some((Ok(event), state));
                    }

                    state.on_telemetry_closed();
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
