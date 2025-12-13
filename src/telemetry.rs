use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::{
    errors::Error,
    types::{Model, StreamEvent, StreamEventKind, Usage},
    RetryMetadata,
};

/// User-provided callbacks for emitting metrics without taking on a tracing dependency.
#[derive(Clone, Default)]
pub struct MetricsCallbacks {
    pub http_request: Option<Arc<dyn Fn(HttpRequestMetrics) + Send + Sync>>,
    pub stream_first_token: Option<Arc<dyn Fn(StreamFirstTokenMetrics) + Send + Sync>>,
    pub usage: Option<Arc<dyn Fn(TokenUsageMetrics) + Send + Sync>>,
}

impl fmt::Debug for MetricsCallbacks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetricsCallbacks")
            .field(
                "http_request",
                &self.http_request.as_ref().map(|_| "callback"),
            )
            .field(
                "stream_first_token",
                &self.stream_first_token.as_ref().map(|_| "callback"),
            )
            .field("usage", &self.usage.as_ref().map(|_| "callback"))
            .finish()
    }
}

/// Common request metadata shared by all telemetry events.
#[derive(Clone, Debug, Default)]
pub struct RequestContext {
    pub method: String,
    pub path: String,
    pub model: Option<Model>,
    pub request_id: Option<String>,
    pub response_id: Option<String>,
}

impl RequestContext {
    pub fn new(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            ..Default::default()
        }
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        if let Some(id) = request_id {
            if !id.trim().is_empty() {
                self.request_id = Some(id);
            }
        }
        self
    }

    pub fn with_response_id(mut self, response_id: Option<String>) -> Self {
        if let Some(id) = response_id {
            if !id.trim().is_empty() {
                self.response_id = Some(id);
            }
        }
        self
    }

    pub fn with_model(mut self, model: Option<Model>) -> Self {
        self.model = model;
        self
    }
}

/// HTTP request latency and outcome.
#[derive(Clone, Debug)]
pub struct HttpRequestMetrics {
    pub latency: Duration,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub retries: Option<RetryMetadata>,
    pub context: RequestContext,
}

/// Time from request start to the first streaming token/event.
#[derive(Clone, Debug)]
pub struct StreamFirstTokenMetrics {
    pub latency: Duration,
    pub error: Option<String>,
    pub context: RequestContext,
}

/// Token usage surfaced once a request/stream finishes.
#[derive(Clone, Debug)]
pub struct TokenUsageMetrics {
    pub usage: Usage,
    pub context: RequestContext,
}

/// Internal helper that owns the registered callbacks (if any).
#[derive(Clone, Default)]
pub(crate) struct Telemetry {
    callbacks: MetricsCallbacks,
}

impl Telemetry {
    pub fn new(callbacks: Option<MetricsCallbacks>) -> Self {
        Self {
            callbacks: callbacks.unwrap_or_default(),
        }
    }

    pub fn http_enabled(&self) -> bool {
        self.callbacks.http_request.is_some()
    }

    pub fn stream_enabled(&self) -> bool {
        self.callbacks.stream_first_token.is_some()
    }

    pub fn usage_enabled(&self) -> bool {
        self.callbacks.usage.is_some()
    }

    pub fn record_http(&self, metrics: HttpRequestMetrics) {
        if let Some(cb) = &self.callbacks.http_request {
            cb(metrics);
        }
    }

    pub fn record_first_token(&self, metrics: StreamFirstTokenMetrics) {
        if let Some(cb) = &self.callbacks.stream_first_token {
            cb(metrics);
        }
    }

    pub fn record_usage(&self, metrics: TokenUsageMetrics) {
        if let Some(cb) = &self.callbacks.usage {
            cb(metrics);
        }
    }

    pub fn stream_state(
        &self,
        context: RequestContext,
        start: Option<Instant>,
    ) -> Option<StreamTelemetry> {
        if self.stream_enabled() || self.usage_enabled() || tracing_enabled() {
            let started = start.unwrap_or_else(Instant::now);
            return Some(StreamTelemetry::new(self.clone(), context, started));
        }
        None
    }
}

/// Shared stream metrics/tracing state passed into async + blocking streaming paths.
#[derive(Clone)]
pub(crate) struct StreamTelemetry {
    inner: Arc<StreamTelemetryInner>,
}

struct StreamTelemetryInner {
    telemetry: Telemetry,
    context: RequestContext,
    start: Instant,
    first_token_recorded: AtomicBool,
}

impl StreamTelemetry {
    pub fn new(telemetry: Telemetry, context: RequestContext, start: Instant) -> Self {
        Self {
            inner: Arc::new(StreamTelemetryInner {
                telemetry,
                context,
                start,
                first_token_recorded: AtomicBool::new(false),
            }),
        }
    }

    pub fn on_event(&self, event: &StreamEvent) {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            event = %event.event_name(),
            kind = %event.kind.as_str(),
            request_id = ?event.request_id,
            response_id = ?event.response_id,
            "stream event"
        );

        if self.inner.telemetry.stream_enabled()
            && matches!(
                event.kind,
                StreamEventKind::MessageDelta
                    | StreamEventKind::MessageStart
                    | StreamEventKind::MessageStop
            )
        {
            self.record_first_token(Some(event), None);
        }

        if self.inner.telemetry.usage_enabled()
            && matches!(event.kind, StreamEventKind::MessageStop)
        {
            if let Some(usage) = &event.usage {
                let ctx = self.context_with_ids(Some(event));
                self.inner.telemetry.record_usage(TokenUsageMetrics {
                    usage: usage.clone(),
                    context: ctx,
                });
            }
        }
    }

    pub fn on_error(&self, error: &Error) {
        #[cfg(feature = "tracing")]
        tracing::warn!(error = %error, "stream error");
        self.record_first_token(None, Some(error.to_string()));
    }

    pub fn on_closed(&self) {
        self.record_first_token(None, Some("stream closed".to_string()));
    }

    fn record_first_token(&self, event: Option<&StreamEvent>, error: Option<String>) {
        if !self.inner.telemetry.stream_enabled() {
            return;
        }
        let already_recorded = self.inner.first_token_recorded.swap(true, Ordering::SeqCst);
        if already_recorded {
            return;
        }
        let latency = self.inner.start.elapsed();
        let ctx = self.context_with_ids(event);
        self.inner
            .telemetry
            .record_first_token(StreamFirstTokenMetrics {
                latency,
                error,
                context: ctx,
            });
    }

    fn context_with_ids(&self, event: Option<&StreamEvent>) -> RequestContext {
        let mut ctx = self.inner.context.clone();
        if let Some(evt) = event {
            if ctx.request_id.is_none() {
                ctx.request_id = evt.request_id.clone();
            }
            if ctx.response_id.is_none() || evt.response_id.is_some() {
                ctx.response_id = evt.response_id.clone();
            }
            if ctx.model.is_none() {
                ctx.model = evt.model.clone();
            }
        }
        ctx
    }
}

pub(crate) fn tracing_enabled() -> bool {
    #[cfg(feature = "tracing")]
    {
        tracing::enabled!(tracing::Level::DEBUG) || tracing::enabled!(tracing::Level::TRACE)
    }
    #[cfg(not(feature = "tracing"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn stream_metrics_capture_first_token_and_usage() {
        let first_calls = Arc::new(Mutex::new(Vec::new()));
        let usage_calls = Arc::new(Mutex::new(Vec::new()));

        let callbacks = MetricsCallbacks {
            stream_first_token: Some({
                let calls = first_calls.clone();
                Arc::new(move |metrics| {
                    calls.lock().unwrap().push(metrics.clone());
                })
            }),
            usage: Some({
                let calls = usage_calls.clone();
                Arc::new(move |metrics| {
                    calls.lock().unwrap().push(metrics.clone());
                })
            }),
            ..Default::default()
        };

        let telemetry = Telemetry::new(Some(callbacks));
        let ctx = RequestContext::new("POST", "/responses")
            .with_model(Some(Model::from("gpt-4o")))
            .with_request_id(Some("req-1".into()));
        let stream = telemetry
            .stream_state(ctx, Some(Instant::now()))
            .expect("stream state");

        let delta = StreamEvent {
            kind: StreamEventKind::MessageDelta,
            event: "message_delta".into(),
            data: None,
            text_delta: Some("hi".into()),
            tool_call_delta: None,
            tool_calls: None,
            response_id: Some("resp-123".into()),
            model: Some(Model::from("gpt-4o")),
            stop_reason: None,
            usage: None,
            request_id: Some("req-1".into()),
            raw: String::new(),
        };

        stream.on_event(&delta);
        assert_eq!(first_calls.lock().unwrap().len(), 1);
        let first = &first_calls.lock().unwrap()[0];
        assert_eq!(first.context.request_id.as_deref(), Some("req-1"));
        assert_eq!(first.context.response_id.as_deref(), Some("resp-123"));

        let mut stop = delta.clone();
        stop.kind = StreamEventKind::MessageStop;
        stop.stop_reason = Some(crate::StopReason::Completed);
        stop.usage = Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
        });

        stream.on_event(&stop);
        let usage = &usage_calls.lock().unwrap()[0];
        assert_eq!(usage.usage.total_tokens, 15);
        assert_eq!(usage.context.response_id.as_deref(), Some("resp-123"));
    }

    #[test]
    fn first_token_error_only_fires_once() {
        let first_calls = Arc::new(Mutex::new(Vec::new()));
        let callbacks = MetricsCallbacks {
            stream_first_token: Some({
                let calls = first_calls.clone();
                Arc::new(move |metrics| {
                    calls.lock().unwrap().push(metrics.clone());
                })
            }),
            ..Default::default()
        };
        let telemetry = Telemetry::new(Some(callbacks));
        let ctx = RequestContext::new("POST", "/responses");
        let stream = telemetry
            .stream_state(ctx, Some(Instant::now()))
            .expect("stream state");

        let err = Error::StreamClosed;
        stream.on_error(&err);
        stream.on_error(&err);

        let calls = first_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].error.as_deref(), Some("stream closed"));
    }
}
