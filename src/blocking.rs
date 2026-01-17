#[cfg(feature = "streaming")]
use std::{collections::VecDeque, io::Read};
#[cfg(feature = "streaming")]
const MAX_PENDING_EVENTS: usize = 512;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use reqwest::{
    blocking::{Client as HttpClient, RequestBuilder, Response as HttpResponse},
    header::{HeaderName, HeaderValue, ACCEPT},
    Method, Url,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::core::RetryState;
#[cfg(feature = "streaming")]
use crate::core::{consume_ndjson_buffer, map_event};
#[cfg(feature = "streaming")]
use crate::telemetry::StreamTelemetry;
#[cfg(feature = "streaming")]
use crate::types::{StopReason, StreamEvent, StreamEventKind, Usage};
use crate::{
    errors::{
        Error, Result, RetryMetadata, StreamTimeoutError, StreamTimeoutKind, TransportError,
        TransportErrorKind, ValidationError,
    },
    generated,
    http::{
        parse_api_error_parts, request_id_from_headers, validate_ndjson_content_type, HeaderList,
        ResponseOptions, RetryConfig, StreamTimeouts,
    },
    runs::{
        RunsCreateOptions, RunsCreateResponse, RunsGetResponse, RunsToolResultsRequest,
        RunsToolResultsResponse,
    },
    telemetry::{HttpRequestMetrics, RequestContext, Telemetry, TokenUsageMetrics},
    types::{CustomerToken, CustomerTokenRequest, Model, Response, ResponseRequest},
    workflow::{RunEventV0, RunId},
    workflow_intent::WorkflowIntentSpec,
    ApiKey, API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_REQUEST_TIMEOUT, REQUEST_ID_HEADER,
};

/// Configuration for the blocking ModelRelay client.
///
/// Use this to configure authentication, timeouts, retries, and other options
/// for the blocking (non-async) client.
#[derive(Clone, Debug, Default)]
pub struct BlockingConfig {
    /// Base URL for the ModelRelay API (defaults to `https://api.modelrelay.ai/api/v1`).
    pub base_url: Option<String>,
    /// API key for authentication (`mr_sk_*` secret key).
    pub api_key: Option<ApiKey>,
    /// Bearer token for authentication (alternative to API key).
    pub access_token: Option<String>,
    /// Custom client identifier sent in headers for debugging/analytics.
    pub client_header: Option<String>,
    /// Custom HTTP client instance (uses default if not provided).
    pub http_client: Option<HttpClient>,
    /// Override the connect timeout (defaults to 5s).
    pub connect_timeout: Option<Duration>,
    /// Override the request timeout (defaults to 60s).
    pub timeout: Option<Duration>,
    /// Retry/backoff policy (defaults to 3 attempts, exponential backoff + jitter).
    pub retry: Option<RetryConfig>,
    /// Default extra headers applied to all requests.
    pub default_headers: Option<HeaderList>,
    /// Optional metrics callbacks (HTTP latency, first-token latency, token usage).
    pub metrics: Option<crate::telemetry::MetricsCallbacks>,
}

/// Blocking (synchronous) client for the ModelRelay API.
///
/// This is the non-async version of [`crate::Client`]. Use this when you need
/// synchronous HTTP calls, such as in CLI tools or contexts without an async runtime.
///
/// # Example
///
/// ```ignore
/// use modelrelay::{BlockingClient, BlockingConfig, ResponseBuilder};
///
/// let client = BlockingClient::new(BlockingConfig {
///     api_key: Some(modelrelay::ApiKey::parse("mr_sk_...")?),
///     ..Default::default()
/// })?;
///
/// let response = ResponseBuilder::new()
///     .model("gpt-4o-mini")
///     .user("Hello!")
///     .send_blocking(&client.responses())?;
/// ```
#[derive(Clone)]
pub struct BlockingClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    base_url: Url,
    api_key: Option<ApiKey>,
    access_token: Option<String>,
    client_header: Option<String>,
    http: HttpClient,
    request_timeout: Duration,
    retry: RetryConfig,
    default_headers: Option<HeaderList>,
    telemetry: Telemetry,
}

/// Blocking NDJSON streaming handle for `POST /responses`.
#[cfg(feature = "streaming")]
pub struct BlockingStreamHandle {
    request_id: Option<String>,
    response: Option<HttpResponse>,
    buffer: String,
    pending: VecDeque<StreamEvent>,
    finished: bool,
    telemetry: Option<StreamTelemetry>,
    stream_timeouts: StreamTimeouts,
    started_at: Instant,
    last_activity: Instant,
    ttft_satisfied: bool,
}

#[cfg(feature = "streaming")]
impl BlockingStreamHandle {
    fn new(
        response: HttpResponse,
        request_id: Option<String>,
        telemetry: Option<StreamTelemetry>,
        stream_timeouts: StreamTimeouts,
        started_at: Instant,
    ) -> Self {
        Self {
            request_id,
            response: Some(response),
            buffer: String::new(),
            pending: VecDeque::new(),
            finished: false,
            telemetry,
            stream_timeouts,
            started_at,
            last_activity: started_at,
            ttft_satisfied: false,
        }
    }

    /// Build a blocking stream handle from pre-baked events (useful for tests/mocks).
    pub fn from_events(events: impl IntoIterator<Item = StreamEvent>) -> Self {
        Self::from_events_with_request_id(events, None)
    }

    /// Build a blocking stream handle from events and an explicit request id.
    pub fn from_events_with_request_id(
        events: impl IntoIterator<Item = StreamEvent>,
        request_id: Option<String>,
    ) -> Self {
        let pending: VecDeque<StreamEvent> = events.into_iter().collect();
        let req_id = request_id.or_else(|| pending.iter().find_map(|evt| evt.request_id.clone()));
        let started_at = Instant::now();
        Self {
            request_id: req_id,
            response: None,
            buffer: String::new(),
            pending,
            finished: false,
            telemetry: None,
            stream_timeouts: StreamTimeouts::default(),
            started_at,
            last_activity: started_at,
            ttft_satisfied: true,
        }
    }

    /// Request identifier returned by the server (if any).
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// Cancel the in-flight streaming request.
    pub fn cancel(&mut self) {
        self.finished = true;
    }

    /// Collect the streaming response into a full `Response`.
    pub fn collect(mut self) -> Result<Response> {
        let mut content = String::new();
        let mut response_id: Option<String> = None;
        let mut model: Option<Model> = None;
        let mut usage: Option<Usage> = None;
        let mut stop_reason: Option<StopReason> = None;
        let mut tool_calls = None;
        let request_id = self.request_id.clone();
        let mut tool_acc = crate::tools::ToolCallAccumulator::default();

        while let Some(evt) = self.next()? {
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
        let output = vec![crate::types::OutputItem::Message {
            role: crate::types::MessageRole::Assistant,
            content: vec![crate::types::ContentPart::text(content)],
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
            decoding_warnings: None,
        })
    }

    /// Pull next streaming event.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<StreamEvent>> {
        if self.finished {
            if let Some(t) = self.telemetry.take() {
                t.on_closed();
            }
            return Ok(None);
        }

        if let Some(total) = self.stream_timeouts.total {
            if self.started_at.elapsed() >= total {
                let err = Error::StreamTimeout(StreamTimeoutError {
                    kind: StreamTimeoutKind::Total,
                    timeout: total,
                });
                if let Some(t) = &self.telemetry {
                    t.on_error(&err);
                }
                self.finished = true;
                return Err(err);
            }
        }
        if !self.ttft_satisfied {
            if let Some(ttft) = self.stream_timeouts.ttft {
                if self.started_at.elapsed() >= ttft {
                    let err = Error::StreamTimeout(StreamTimeoutError {
                        kind: StreamTimeoutKind::Ttft,
                        timeout: ttft,
                    });
                    if let Some(t) = &self.telemetry {
                        t.on_error(&err);
                    }
                    self.finished = true;
                    return Err(err);
                }
            }
        }

        if let Some(evt) = self.pending.pop_front() {
            if let Some(t) = &self.telemetry {
                t.on_event(&evt);
            }
            return Ok(Some(evt));
        }
        if self.response.is_none() {
            self.finished = true;
            if let Some(t) = self.telemetry.take() {
                t.on_closed();
            }
            return Ok(None);
        }

        let mut buf = [0u8; 4096];
        loop {
            if let Some(evt) = self.pending.pop_front() {
                if let Some(t) = &self.telemetry {
                    t.on_event(&evt);
                }
                return Ok(Some(evt));
            }

            if let Some(idle) = self.stream_timeouts.idle {
                if self.last_activity.elapsed() >= idle {
                    let err = Error::StreamTimeout(StreamTimeoutError {
                        kind: StreamTimeoutKind::Idle,
                        timeout: idle,
                    });
                    if let Some(t) = &self.telemetry {
                        t.on_error(&err);
                    }
                    self.finished = true;
                    return Err(err);
                }
            }
            let read = self
                .response
                .as_mut()
                .expect("response must exist when reading")
                .read(&mut buf)
                .map_err(|err| {
                    let error = Error::Transport(TransportError {
                        kind: TransportErrorKind::Request,
                        message: err.to_string(),
                        source: None,
                        retries: None,
                    });
                    if let Some(t) = &self.telemetry {
                        t.on_error(&error);
                    }
                    error
                })?;
            if read == 0 {
                let (events, _) = consume_ndjson_buffer(&self.buffer);
                self.buffer.clear();
                for raw in events {
                    match map_event(raw, self.request_id.clone()) {
                        Ok(Some(evt)) => self.pending.push_back(evt),
                        Ok(None) => {} // keepalive, skip
                        Err(err) => {
                            if let Some(t) = &self.telemetry {
                                t.on_error(&err);
                            }
                            return Err(err);
                        }
                    }
                }
                self.finished = true;
                if let Some(evt) = self.pending.pop_front() {
                    if let Some(t) = &self.telemetry {
                        t.on_event(&evt);
                    }
                    return Ok(Some(evt));
                }
                if let Some(t) = self.telemetry.take() {
                    t.on_closed();
                }
                return Ok(None);
            }
            self.last_activity = Instant::now();
            let chunk = String::from_utf8_lossy(&buf[..read]);
            self.buffer.push_str(&chunk);
            let (events, remainder) = consume_ndjson_buffer(&self.buffer);
            self.buffer = remainder;
            for raw in events {
                match map_event(raw, self.request_id.clone()) {
                    Ok(Some(evt)) => {
                        if !self.ttft_satisfied && blocking_event_counts_for_ttft(&evt) {
                            self.ttft_satisfied = true;
                        }
                        self.pending.push_back(evt);
                        if self.pending.len() > MAX_PENDING_EVENTS {
                            let err = Error::StreamBackpressure {
                                dropped: self.pending.len(),
                            };
                            if let Some(t) = &self.telemetry {
                                t.on_error(&err);
                            }
                            return Err(err);
                        }
                    }
                    Ok(None) => {} // keepalive, skip
                    Err(err) => {
                        if let Some(t) = &self.telemetry {
                            t.on_error(&err);
                        }
                        return Err(err);
                    }
                }
            }
        }
    }
}

#[cfg(feature = "streaming")]
fn blocking_event_counts_for_ttft(evt: &StreamEvent) -> bool {
    if let Some(text) = &evt.text_delta {
        if !text.is_empty() {
            return true;
        }
    }
    if evt.tool_call_delta.is_some() {
        return true;
    }
    if evt
        .tool_calls
        .as_ref()
        .map(|c| !c.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    evt.event == "error"
}

#[cfg(feature = "streaming")]
impl Drop for BlockingStreamHandle {
    fn drop(&mut self) {
        self.finished = true;
        if let Some(t) = self.telemetry.take() {
            t.on_closed();
        }
    }
}

impl BlockingClient {
    /// Create a new blocking client with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid (e.g., malformed base URL).
    pub fn new(cfg: BlockingConfig) -> Result<Self> {
        let base_source = cfg
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        // Ensure trailing slash so joins keep "/api/v1/".
        let base = format!("{}/", base_source.trim_end_matches('/'));
        let base_url = Url::parse(&base)
            .map_err(|err| Error::Validation(format!("invalid base url: {err}").into()))?;

        let connect_timeout = cfg.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT);
        let request_timeout = cfg.timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT);
        let retry = cfg.retry.unwrap_or_default();

        let http = match cfg.http_client {
            Some(client) => client,
            None => HttpClient::builder()
                .connect_timeout(connect_timeout)
                .build()
                .map_err(|err| TransportError {
                    kind: TransportErrorKind::Connect,
                    message: "failed to build http client".to_string(),
                    source: Some(err),
                    retries: None,
                })?,
        };

        let client_header = cfg
            .client_header
            .filter(|s| !s.trim().is_empty())
            .or_else(|| Some(DEFAULT_CLIENT_HEADER.to_string()));

        Ok(Self {
            inner: Arc::new(ClientInner {
                base_url,
                api_key: cfg.api_key,
                access_token: cfg.access_token.filter(|s| !s.trim().is_empty()),
                client_header,
                http,
                request_timeout,
                retry,
                default_headers: cfg.default_headers,
                telemetry: Telemetry::new(cfg.metrics),
            }),
        })
    }

    /// Returns the Responses client for `POST /responses` (streaming + non-streaming).
    pub fn responses(&self) -> BlockingResponsesClient {
        BlockingResponsesClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the auth client for customer token operations.
    pub fn auth(&self) -> BlockingAuthClient {
        BlockingAuthClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the runs client for workflow runs (`/runs`).
    pub fn runs(&self) -> BlockingRunsClient {
        BlockingRunsClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the billing client for customer self-service operations.
    ///
    /// These endpoints require a customer bearer token.
    /// API keys are not accepted.
    #[cfg(feature = "billing")]
    pub fn billing(&self) -> BlockingBillingClient {
        BlockingBillingClient {
            inner: self.inner.clone(),
        }
    }
}

/// Blocking client for customer token operations.
#[derive(Clone)]
pub struct BlockingAuthClient {
    inner: Arc<ClientInner>,
}

impl BlockingAuthClient {
    /// Mint a customer-scoped bearer token (requires secret key auth).
    pub fn customer_token(&self, req: CustomerTokenRequest) -> Result<CustomerToken> {
        let has_customer_id = req.customer_id.is_some();
        let has_external = req
            .customer_external_id
            .as_ref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        if has_customer_id == has_external {
            return Err(Error::Validation(ValidationError::new(
                "provide exactly one of customer_id or customer_external_id",
            )));
        }

        let mut builder = self
            .inner
            .request(Method::POST, "/auth/customer-token")?
            .json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;

        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::POST, "/auth/customer-token", None, None);
        self.inner.execute_json(builder, Method::POST, None, ctx)
    }
}

/// Blocking client for `POST /responses`.
///
/// Prefer using [`ResponseBuilder`] for ergonomic request construction.
///
/// [`ResponseBuilder`]: crate::ResponseBuilder
#[derive(Clone)]
pub struct BlockingResponsesClient {
    inner: Arc<ClientInner>,
}

impl BlockingResponsesClient {
    /// Streaming handle over NDJSON events (blocking client).
    #[cfg(feature = "streaming")]
    pub(crate) fn stream(
        &self,
        req: ResponseRequest,
        options: ResponseOptions,
    ) -> Result<BlockingStreamHandle> {
        self.inner.ensure_auth()?;
        let mut require_model = !options.headers.iter().any(|h| {
            h.key
                .eq_ignore_ascii_case(crate::responses::CUSTOMER_ID_HEADER)
        });
        if require_model && self.inner.has_jwt_access_token() {
            require_model = false;
        }
        req.validate(require_model)?;
        let mut builder = self.inner.request(Method::POST, "/responses")?.json(&req);
        builder = self.inner.with_headers(
            builder,
            options.request_id.as_deref(),
            &options.headers,
            Some(crate::responses::RESPONSES_STREAM_ACCEPT),
        )?;
        let retry = options
            .retry
            .clone()
            .unwrap_or_else(|| self.inner.retry.clone());
        let mut ctx = self.inner.make_context(
            &Method::POST,
            "/responses",
            req.model.clone(),
            options.request_id.clone(),
        );
        let stream_start = Instant::now();
        let resp = self
            .inner
            .send_with_retry(builder, Method::POST, retry, ctx.clone())?;

        validate_ndjson_content_type(resp.headers(), resp.status().as_u16())?;

        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);
        ctx = ctx.with_request_id(request_id.clone());
        let stream_telemetry = self.inner.telemetry.stream_state(ctx, Some(stream_start));
        Ok(BlockingStreamHandle::new(
            resp,
            request_id,
            stream_telemetry,
            options.stream_timeouts,
            stream_start,
        ))
    }

    pub(crate) fn create(
        &self,
        req: ResponseRequest,
        options: ResponseOptions,
    ) -> Result<Response> {
        self.inner.ensure_auth()?;
        let mut require_model = !options.headers.iter().any(|h| {
            h.key
                .eq_ignore_ascii_case(crate::responses::CUSTOMER_ID_HEADER)
        });
        if require_model && self.inner.has_jwt_access_token() {
            require_model = false;
        }
        req.validate(require_model)?;
        let mut builder = self.inner.request(Method::POST, "/responses")?.json(&req);
        builder = self.inner.with_headers(
            builder,
            options.request_id.as_deref(),
            &options.headers,
            Some("application/json"),
        )?;

        builder = self.inner.with_timeout(builder, options.timeout, true);
        let retry = options
            .retry
            .clone()
            .unwrap_or_else(|| self.inner.retry.clone());

        let ctx = self.inner.make_context(
            &Method::POST,
            "/responses",
            req.model.clone(),
            options.request_id.clone(),
        );
        let resp = self
            .inner
            .send_with_retry(builder, Method::POST, retry, ctx)?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        let bytes = resp
            .bytes()
            .map_err(|err| self.inner.to_transport_error(err, None))?;
        let mut payload: Response = serde_json::from_slice(&bytes).map_err(Error::Serialization)?;
        payload.request_id = request_id;
        if self.inner.telemetry.usage_enabled() {
            let ctx = RequestContext::new(Method::POST.as_str(), "/responses")
                .with_model(Some(payload.model.clone()))
                .with_request_id(payload.request_id.clone())
                .with_response_id(Some(payload.id.clone()));
            self.inner.telemetry.record_usage(TokenUsageMetrics {
                usage: payload.usage.clone(),
                context: ctx,
            });
        }
        Ok(payload)
    }

    /// Create a structured response with schema inference + validation retries (blocking).
    pub fn create_structured<T, H>(
        &self,
        builder: crate::responses::ResponseBuilder,
        options: crate::structured::StructuredOptions<H>,
    ) -> std::result::Result<
        crate::structured::StructuredResult<T>,
        crate::structured::StructuredError,
    >
    where
        T: JsonSchema + DeserializeOwned,
        H: crate::structured::RetryHandler,
    {
        let output_format =
            crate::structured::output_format_from_type::<T>(options.schema_name.as_deref())?;
        let mut inner = builder.output_format(output_format);
        let mut executor = crate::structured::RetryExecutor::new(&options);

        loop {
            let response: Response = inner
                .clone()
                .send_blocking(self)
                .map_err(crate::structured::StructuredError::Sdk)?;
            match executor.process_response::<T>(&response, &inner.payload.input)? {
                crate::structured::RetryDecision::Success(result) => return Ok(result),
                crate::structured::RetryDecision::Exhausted(err) => {
                    return Err(crate::structured::StructuredError::Exhausted(err));
                }
                crate::structured::RetryDecision::Retry(retry_items) => {
                    for item in retry_items {
                        inner = inner.item(item);
                    }
                }
            }
        }
    }

    /// Convenience helper for the common "system + user -> assistant text" path (blocking).
    ///
    /// This is a thin wrapper around `ResponseBuilder` and `Response::text()`.
    /// Returns an `EmptyResponse` transport error if the response contains no
    /// assistant text output.
    pub fn text(
        &self,
        model: impl Into<crate::types::Model>,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Result<String> {
        crate::responses::ResponseBuilder::text_prompt(system, user)
            .model(model)
            .send_text_blocking(self)
    }

    /// Convenience helper for customer-attributed requests where the backend selects the model.
    ///
    /// This sets the customer id header and omits `model` from the request body.
    pub fn text_for_customer(
        &self,
        customer_id: impl Into<String>,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Result<String> {
        crate::responses::ResponseBuilder::text_prompt(system, user)
            .customer_id(customer_id)
            .send_text_blocking(self)
    }

    /// Convenience helper to stream text deltas directly (blocking).
    #[cfg(feature = "streaming")]
    pub fn stream_text_deltas(
        &self,
        model: impl Into<crate::types::Model>,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Result<impl Iterator<Item = Result<String>>> {
        crate::responses::ResponseBuilder::text_prompt(system, user)
            .model(model)
            .stream_text_deltas_blocking(self)
    }
}

/// Blocking client for workflow runs (`/runs`).
#[derive(Clone)]
pub struct BlockingRunsClient {
    inner: Arc<ClientInner>,
}

#[derive(serde::Serialize)]
struct RunsCreateRequest {
    spec: WorkflowIntentSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<std::collections::HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_overrides: Option<crate::runs::RunsModelOverrides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<RunsCreateOptionsV0>,
}

#[derive(serde::Serialize)]
struct RunsCreateOptionsV0 {
    idempotency_key: String,
}

#[cfg(feature = "streaming")]
pub struct BlockingRunEventStreamHandle {
    request_id: Option<String>,
    finished: bool,
    de: serde_json::Deserializer<serde_json::de::IoRead<std::io::BufReader<HttpResponse>>>,
}

#[cfg(feature = "streaming")]
impl BlockingRunEventStreamHandle {
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
}

#[cfg(feature = "streaming")]
impl Iterator for BlockingRunEventStreamHandle {
    type Item = Result<RunEventV0>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match <RunEventV0 as serde::Deserialize>::deserialize(&mut self.de) {
            Ok(ev) => match ev.validate() {
                Ok(()) => Some(Ok(ev)),
                Err(err) => {
                    self.finished = true;
                    Some(Err(Error::StreamProtocol {
                        message: format!("invalid run event: {err}"),
                        raw_data: None,
                    }))
                }
            },
            Err(err) => {
                if err.is_eof() {
                    self.finished = true;
                    return None;
                }
                self.finished = true;
                Some(Err(Error::StreamProtocol {
                    message: format!("failed to parse run event: {err}"),
                    raw_data: None,
                }))
            }
        }
    }
}

impl BlockingRunsClient {
    pub fn create(&self, spec: WorkflowIntentSpec) -> Result<RunsCreateResponse> {
        self.create_with_options(spec, RunsCreateOptions::default())
    }

    pub fn create_with_options(
        &self,
        spec: WorkflowIntentSpec,
        options: RunsCreateOptions,
    ) -> Result<RunsCreateResponse> {
        self.inner.ensure_auth()?;
        if options.session_id.is_some_and(|id| id.is_nil()) {
            return Err(Error::Validation(
                ValidationError::new("session_id is required").with_field("session_id"),
            ));
        }
        let options_payload = options.idempotency_key.as_ref().and_then(|key| {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(RunsCreateOptionsV0 {
                    idempotency_key: trimmed.to_string(),
                })
            }
        });

        let model_override = options
            .model_override
            .as_ref()
            .and_then(|val| match val.trim() {
                "" => None,
                trimmed => Some(trimmed.to_string()),
            });
        let model_overrides = options.model_overrides.clone();

        let mut builder = self.inner.request(Method::POST, "/runs")?;
        builder = builder.json(&RunsCreateRequest {
            spec,
            input: options.input,
            model_override,
            model_overrides,
            session_id: options.session_id,
            options: options_payload,
        });
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, "/runs", None, None);
        self.inner.execute_json(builder, Method::POST, None, ctx)
    }

    pub fn create_with_session(
        &self,
        spec: WorkflowIntentSpec,
        session_id: Uuid,
    ) -> Result<RunsCreateResponse> {
        self.create_with_options(
            spec,
            RunsCreateOptions {
                session_id: Some(session_id),
                ..RunsCreateOptions::default()
            },
        )
    }

    pub fn get(&self, run_id: RunId) -> Result<RunsGetResponse> {
        self.inner.ensure_auth()?;
        if run_id.0.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("run_id is required").with_field("run_id"),
            ));
        }
        let path = format!("/runs/{}", run_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        self.inner.execute_json(builder, Method::GET, None, ctx)
    }

    pub fn submit_tool_results(
        &self,
        run_id: RunId,
        req: RunsToolResultsRequest,
    ) -> Result<RunsToolResultsResponse> {
        self.inner.ensure_auth()?;
        if run_id.0.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("run_id is required").with_field("run_id"),
            ));
        }
        let path = format!("/runs/{}/tool-results", run_id);
        let mut builder = self.inner.request(Method::POST, &path)?;
        builder = builder.json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, &path, None, None);
        self.inner.execute_json(builder, Method::POST, None, ctx)
    }

    #[cfg(feature = "streaming")]
    pub fn stream_events(
        &self,
        run_id: RunId,
        after_seq: Option<i64>,
        limit: Option<i64>,
    ) -> Result<BlockingRunEventStreamHandle> {
        self.inner.ensure_auth()?;
        if run_id.0.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("run_id is required").with_field("run_id"),
            ));
        }
        let mut path = format!("/runs/{}/events", run_id);
        let mut q = vec![];
        if let Some(seq) = after_seq {
            if seq > 0 {
                q.push(("after_seq", seq.to_string()));
            }
        }
        if let Some(lim) = limit {
            if lim > 0 {
                q.push(("limit", lim.to_string()));
            }
        }
        if !q.is_empty() {
            path.push('?');
            path.push_str(
                &q.into_iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }

        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/x-ndjson"),
        )?;
        // No request timeout for streaming.
        let builder = self.inner.with_timeout(builder, None, false);
        let retry = self.inner.retry.clone();
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp = self
            .inner
            .send_with_retry(builder, Method::GET, retry, ctx)?;

        validate_ndjson_content_type(resp.headers(), resp.status().as_u16())?;

        let request_id = request_id_from_headers(resp.headers());
        let reader = std::io::BufReader::new(resp);
        let de = serde_json::Deserializer::from_reader(reader);
        Ok(BlockingRunEventStreamHandle {
            request_id,
            finished: false,
            de,
        })
    }
}

/// Blocking client for customer billing self-service operations.
///
/// These endpoints require a customer bearer token.
/// API keys are not accepted.
#[cfg(feature = "billing")]
#[derive(Clone)]
pub struct BlockingBillingClient {
    inner: Arc<ClientInner>,
}

#[cfg(feature = "billing")]
impl BlockingBillingClient {
    /// Get the authenticated customer's profile.
    ///
    /// Returns customer details including ID, email, external ID, and metadata.
    pub fn me(&self) -> Result<generated::CustomerMe> {
        let path = "/customers/me";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        let response: generated::CustomerMeResponse =
            self.inner.execute_json(builder, Method::GET, None, ctx)?;
        Ok(response.customer)
    }

    /// Get the authenticated customer's subscription details.
    ///
    /// Returns subscription status, tier information, and billing provider.
    pub fn subscription(&self) -> Result<generated::CustomerMeSubscription> {
        let path = "/customers/me/subscription";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        let response: generated::CustomerMeSubscriptionResponse =
            self.inner.execute_json(builder, Method::GET, None, ctx)?;
        Ok(response.subscription)
    }

    /// Get the authenticated customer's usage metrics.
    ///
    /// Returns token usage, request counts, and cost for the current billing window.
    pub fn usage(&self) -> Result<generated::CustomerMeUsage> {
        let path = "/customers/me/usage";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        let response: generated::CustomerMeUsageResponse =
            self.inner.execute_json(builder, Method::GET, None, ctx)?;
        Ok(response.usage)
    }

    /// Get the authenticated customer's credit balance.
    ///
    /// For PAYGO subscriptions, returns the current balance and reserved amount.
    pub fn balance(&self) -> Result<generated::CustomerBalanceResponse> {
        let path = "/customers/me/balance";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        self.inner.execute_json(builder, Method::GET, None, ctx)
    }

    /// Get the authenticated customer's balance transaction history.
    ///
    /// Returns a list of ledger entries showing credits and debits.
    pub fn balance_history(&self) -> Result<generated::CustomerLedgerResponse> {
        let path = "/customers/me/balance/history";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        self.inner.execute_json(builder, Method::GET, None, ctx)
    }

    /// Create a top-up checkout session.
    ///
    /// For PAYGO subscriptions, creates a Stripe Checkout session to add credits.
    pub fn topup(
        &self,
        req: generated::CustomerTopupRequest,
    ) -> Result<generated::CustomerTopupResponse> {
        let path = "/customers/me/topup";
        let mut builder = self.inner.request(Method::POST, path)?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, path, None, None);
        self.inner.execute_json(builder, Method::POST, None, ctx)
    }

    /// Change the authenticated customer's subscription tier.
    ///
    /// Switches to a different tier within the same project.
    pub fn change_tier(&self, tier_code: &str) -> Result<generated::CustomerMeSubscription> {
        let path = "/customers/me/change-tier";
        let req = generated::ChangeTierRequest {
            tier_code: tier_code.to_string(),
        };
        let mut builder = self.inner.request(Method::POST, path)?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, path, None, None);
        let response: generated::CustomerMeSubscriptionResponse =
            self.inner.execute_json(builder, Method::POST, None, ctx)?;
        Ok(response.subscription)
    }

    /// Create a subscription checkout session.
    ///
    /// Creates a Stripe Checkout session for subscribing to a tier.
    pub fn checkout(
        &self,
        req: generated::CustomerMeCheckoutRequest,
    ) -> Result<generated::CheckoutSessionResponse> {
        let path = "/customers/me/checkout";
        let mut builder = self.inner.request(Method::POST, path)?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, path, None, None);
        self.inner.execute_json(builder, Method::POST, None, ctx)
    }
}

impl ClientInner {
    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            Url::parse(path).map_err(|err| Error::Validation(err.to_string().into()))?
        } else {
            let rel = path.trim_start_matches('/');
            self.base_url
                .join(rel)
                .map_err(|err| Error::Validation(format!("invalid path: {err}").into()))?
        };
        Ok(self.http.request(method, url))
    }

    fn with_headers(
        &self,
        mut builder: RequestBuilder,
        request_id: Option<&str>,
        headers: &HeaderList,
        accept: Option<&str>,
    ) -> Result<RequestBuilder> {
        if let Some(accept) = accept {
            builder = builder.header(ACCEPT, accept);
        }
        if let Some(req_id) = request_id {
            if !req_id.trim().is_empty() {
                builder = builder.header(REQUEST_ID_HEADER, req_id);
            }
        }
        if let Some(client_header) = self.client_header.as_deref() {
            builder = builder.header("X-ModelRelay-Client", client_header);
        }
        builder = self.apply_auth(builder);

        if let Some(defaults) = &self.default_headers {
            builder = apply_header_list(builder, defaults)?;
        }
        builder = apply_header_list(builder, headers)?;

        Ok(builder)
    }

    fn with_timeout(
        &self,
        builder: RequestBuilder,
        timeout: Option<Duration>,
        use_default: bool,
    ) -> RequestBuilder {
        if let Some(duration) = timeout {
            builder.timeout(duration)
        } else if use_default {
            builder.timeout(self.request_timeout)
        } else {
            builder
        }
    }

    fn ensure_auth(&self) -> Result<()> {
        if self.api_key.is_some()
            || self
                .access_token
                .as_ref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
        {
            return Ok(());
        }
        Err(Error::Validation(ValidationError::new(
            "api key or access token is required",
        )))
    }

    fn has_jwt_access_token(&self) -> bool {
        self.access_token
            .as_deref()
            .map(crate::core::is_jwt_token)
            .unwrap_or(false)
    }

    fn make_context(
        &self,
        method: &Method,
        path: &str,
        model: Option<Model>,
        request_id: Option<String>,
    ) -> RequestContext {
        RequestContext::new(method.as_str(), path)
            .with_model(model)
            .with_request_id(request_id)
    }

    fn apply_auth(&self, mut builder: RequestBuilder) -> RequestBuilder {
        if let Some(token) = &self.access_token {
            let bearer = token
                .trim()
                .strip_prefix("Bearer ")
                .or_else(|| token.trim().strip_prefix("bearer "))
                .unwrap_or(token.trim());
            builder = builder.bearer_auth(bearer.to_string());
        }
        if let Some(key) = &self.api_key {
            builder = builder.header(API_KEY_HEADER, key.as_str());
        }
        builder
    }

    fn execute_json<T: DeserializeOwned>(
        &self,
        builder: RequestBuilder,
        method: Method,
        retry: Option<RetryConfig>,
        ctx: RequestContext,
    ) -> Result<T> {
        let retry_cfg = retry.unwrap_or_else(|| self.retry.clone());
        let resp = self.send_with_retry(builder, method, retry_cfg, ctx)?;
        let bytes = resp
            .bytes()
            .map_err(|err| self.to_transport_error(err, None))?;
        let parsed = serde_json::from_slice::<T>(&bytes).map_err(Error::Serialization)?;
        Ok(parsed)
    }

    fn send_with_retry(
        &self,
        builder: RequestBuilder,
        method: Method,
        retry: RetryConfig,
        ctx: RequestContext,
    ) -> Result<HttpResponse> {
        let max_attempts = retry.max_attempts.max(1);
        let mut state = RetryState::new();
        let start = Instant::now();

        for attempt in 1..=max_attempts {
            let attempt_builder = builder.try_clone().ok_or_else(|| {
                Error::Validation("request body is not cloneable for retry".into())
            })?;
            #[cfg(feature = "tracing")]
            let span = tracing::debug_span!(
                "modelrelay.http",
                method = %ctx.method,
                path = %ctx.path,
                attempt,
                max_attempts
            );
            #[cfg(feature = "tracing")]
            let _guard = span.enter();
            let result = attempt_builder.send();

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let mut http_ctx = ctx.clone();
                        if http_ctx.request_id.is_none() {
                            http_ctx.request_id =
                                request_id_from_headers(resp.headers()).or(http_ctx.request_id);
                        }
                        if self.telemetry.http_enabled() {
                            self.telemetry.record_http(HttpRequestMetrics {
                                latency: start.elapsed(),
                                status: Some(status.as_u16()),
                                error: None,
                                retries: state.metadata(),
                                context: http_ctx,
                            });
                        }
                        #[cfg(feature = "tracing")]
                        tracing::debug!(
                            status = %status,
                            elapsed_ms = start.elapsed().as_millis() as u64,
                            "request completed"
                        );
                        return Ok(resp);
                    }
                    state.record_attempt(attempt);
                    state.record_status(status);

                    let should_retry = retry.should_retry_status(&method, status);
                    if should_retry && attempt < max_attempts {
                        thread::sleep(retry.backoff_delay(attempt));
                        continue;
                    }

                    let retries = state.metadata();
                    let headers = resp.headers().clone();
                    let mut http_ctx = ctx.clone();
                    if http_ctx.request_id.is_none() {
                        http_ctx.request_id =
                            request_id_from_headers(&headers).or(http_ctx.request_id);
                    }
                    if self.telemetry.http_enabled() {
                        self.telemetry.record_http(HttpRequestMetrics {
                            latency: start.elapsed(),
                            status: Some(status.as_u16()),
                            error: Some(format!("http {}", status.as_u16())),
                            retries: retries.clone(),
                            context: http_ctx,
                        });
                    }
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        status = %status,
                        attempt,
                        "request failed; returning error"
                    );
                    let body = match resp.text() {
                        Ok(text) => text,
                        Err(e) => format!("[failed to read response body: {}]", e),
                    };
                    return Err(parse_api_error_parts(status, &headers, body, retries));
                }
                Err(err) => {
                    state.record_attempt(attempt);
                    state.record_error(&err);
                    let should_retry = retry.should_retry_error(&method, &err);
                    if should_retry && attempt < max_attempts {
                        thread::sleep(retry.backoff_delay(attempt));
                        continue;
                    }

                    let retries = state.metadata();
                    if self.telemetry.http_enabled() {
                        self.telemetry.record_http(HttpRequestMetrics {
                            latency: start.elapsed(),
                            status: None,
                            error: Some(err.to_string()),
                            retries: retries.clone(),
                            context: ctx.clone(),
                        });
                    }
                    #[cfg(feature = "tracing")]
                    tracing::warn!(attempt, error = %err, "transport error");
                    return Err(self.to_transport_error(err, retries));
                }
            }
        }

        Err(Error::Transport(TransportError {
            kind: TransportErrorKind::Other,
            message: "request failed".to_string(),
            source: None,
            retries: state.metadata(),
        }))
    }

    fn to_transport_error(&self, err: reqwest::Error, retries: Option<RetryMetadata>) -> Error {
        TransportError {
            kind: crate::ndjson::classify_reqwest_error(&err),
            message: err.to_string(),
            source: Some(err),
            retries,
        }
        .into()
    }
}

fn apply_header_list(mut builder: RequestBuilder, headers: &HeaderList) -> Result<RequestBuilder> {
    for entry in headers.iter() {
        if !entry.is_valid() {
            continue;
        }
        let name = HeaderName::from_bytes(entry.key.trim().as_bytes())
            .map_err(|err| Error::Validation(format!("invalid header name: {err}").into()))?;
        let val = HeaderValue::from_str(entry.value.trim())
            .map_err(|err| Error::Validation(format!("invalid header value: {err}").into()))?;
        builder = builder.header(name, val);
    }
    Ok(builder)
}

// RetryState, RawEvent, consume_ndjson_buffer, and map_event are now in core.rs
