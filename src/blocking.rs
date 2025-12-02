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
    Method, StatusCode, Url,
    blocking::{Client as HttpClient, RequestBuilder, Response},
    header::{ACCEPT, HeaderName, HeaderValue},
};
use serde::de::DeserializeOwned;
use serde_json;

#[cfg(all(feature = "blocking", feature = "streaming"))]
use crate::chat::ChatStreamAdapter;
#[cfg(feature = "streaming")]
use crate::telemetry::StreamTelemetry;
#[cfg(feature = "streaming")]
use crate::types::{StopReason, StreamEvent, StreamEventKind, Usage};
use crate::{
    API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_REQUEST_TIMEOUT, REQUEST_ID_HEADER,
    errors::{Error, Result, RetryMetadata, TransportError, TransportErrorKind, ValidationError},
    http::{
        HeaderList, ProxyOptions, RetryConfig, StreamFormat, parse_api_error_parts,
        request_id_from_headers,
    },
    telemetry::{HttpRequestMetrics, RequestContext, Telemetry, TokenUsageMetrics},
    types::{
        APIKey, FrontendToken, FrontendTokenRequest, Model, Provider, ProxyRequest, ProxyResponse,
    },
};

#[derive(Clone, Debug, Default)]
pub struct BlockingConfig {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub client_header: Option<String>,
    pub http_client: Option<HttpClient>,
    /// Override the connect timeout (defaults to 5s).
    pub connect_timeout: Option<Duration>,
    /// Override the request timeout (defaults to 60s).
    pub timeout: Option<Duration>,
    /// Retry/backoff policy (defaults to 3 attempts, exponential backoff + jitter).
    pub retry: Option<RetryConfig>,
    /// Default extra headers applied to all requests.
    pub default_headers: Option<HeaderList>,
    /// Default metadata applied to all proxy requests.
    pub default_metadata: Option<HeaderList>,
    /// Optional metrics callbacks (HTTP latency, first-token latency, token usage).
    pub metrics: Option<crate::telemetry::MetricsCallbacks>,
}

#[derive(Clone)]
pub struct BlockingClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    base_url: Url,
    api_key: Option<String>,
    access_token: Option<String>,
    client_header: Option<String>,
    http: HttpClient,
    request_timeout: Duration,
    retry: RetryConfig,
    default_headers: Option<HeaderList>,
    default_metadata: Option<HeaderList>,
    telemetry: Telemetry,
}

/// Blocking SSE streaming handle for LLM proxy.
#[cfg(feature = "streaming")]
pub struct BlockingProxyHandle {
    request_id: Option<String>,
    response: Option<Response>,
    buffer: String,
    pending: VecDeque<StreamEvent>,
    finished: bool,
    stream_format: StreamFormat,
    telemetry: Option<StreamTelemetry>,
}

#[cfg(feature = "streaming")]
impl BlockingProxyHandle {
    fn new(
        response: Response,
        request_id: Option<String>,
        stream_format: StreamFormat,
        telemetry: Option<StreamTelemetry>,
    ) -> Self {
        Self {
            request_id,
            response: Some(response),
            buffer: String::new(),
            pending: VecDeque::new(),
            finished: false,
            stream_format,
            telemetry,
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
        Self {
            request_id: req_id,
            response: None,
            buffer: String::new(),
            pending,
            finished: false,
            stream_format: StreamFormat::Sse,
            telemetry: None,
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

    /// Collect the streaming response into a full `ProxyResponse`.
    pub fn collect(mut self) -> Result<ProxyResponse> {
        let mut content = String::new();
        let mut response_id: Option<String> = None;
        let mut model: Option<Model> = None;
        let mut usage: Option<Usage> = None;
        let mut stop_reason: Option<StopReason> = None;
        let request_id = self.request_id.clone();

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
            provider: Provider::Other("stream".to_string()),
            id: response_id
                .or_else(|| request_id.clone())
                .unwrap_or_else(|| "stream".to_string()),
            content: vec![content],
            stop_reason,
            model: model.unwrap_or_else(|| Model::Other(String::new())),
            usage: usage.unwrap_or_default(),
            request_id,
        })
    }

    /// Pull next streaming event.
    pub fn next(&mut self) -> Result<Option<StreamEvent>> {
        if self.finished {
            if let Some(t) = self.telemetry.take() {
                t.on_closed();
            }
            return Ok(None);
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
                let (events, _) = if self.stream_format == StreamFormat::Ndjson {
                    consume_ndjson_buffer(&self.buffer)
                } else {
                    consume_sse_buffer(&self.buffer, true)
                };
                self.buffer.clear();
                for raw in events {
                    if let Some(evt) = map_event(raw, self.request_id.clone()) {
                        self.pending.push_back(evt);
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
            let chunk = String::from_utf8_lossy(&buf[..read]);
            self.buffer.push_str(&chunk);
            let (events, remainder) = if self.stream_format == StreamFormat::Ndjson {
                consume_ndjson_buffer(&self.buffer)
            } else {
                consume_sse_buffer(&self.buffer, false)
            };
            self.buffer = remainder;
            for raw in events {
                if let Some(evt) = map_event(raw, self.request_id.clone()) {
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
            }
        }
    }
}

#[cfg(feature = "streaming")]
impl Drop for BlockingProxyHandle {
    fn drop(&mut self) {
        self.finished = true;
        if let Some(t) = self.telemetry.take() {
            t.on_closed();
        }
    }
}

impl BlockingClient {
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
                api_key: cfg.api_key.filter(|s| !s.trim().is_empty()),
                access_token: cfg.access_token.filter(|s| !s.trim().is_empty()),
                client_header,
                http,
                request_timeout,
                retry,
                default_headers: cfg.default_headers,
                default_metadata: cfg.default_metadata,
                telemetry: Telemetry::new(cfg.metrics),
            }),
        })
    }

    pub fn llm(&self) -> BlockingLLMClient {
        BlockingLLMClient {
            inner: self.inner.clone(),
        }
    }

    pub fn auth(&self) -> BlockingAuthClient {
        BlockingAuthClient {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Clone)]
pub struct BlockingAuthClient {
    inner: Arc<ClientInner>,
}

impl BlockingAuthClient {
    pub fn frontend_token(&self, req: FrontendTokenRequest) -> Result<FrontendToken> {
        if req.customer_id.is_none() {
            return Err(Error::Validation(
                ValidationError::new("customer_id is required").with_field("customer_id"),
            ));
        }
        if req
            .publishable_key
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            return Err(Error::Validation(
                ValidationError::new("publishable key is required").with_field("publishable_key"),
            ));
        }

        let mut builder = self
            .inner
            .request(Method::POST, "/auth/frontend-token")?
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
            .make_context(&Method::POST, "/auth/frontend-token", None, None, None);
        self.inner.execute_json(builder, Method::POST, None, ctx)
    }
}

#[derive(Clone)]
pub struct BlockingLLMClient {
    inner: Arc<ClientInner>,
}

impl BlockingLLMClient {
    /// Streaming handle over SSE chat events (blocking client).
    #[cfg(feature = "streaming")]
    pub fn proxy_stream(
        &self,
        req: ProxyRequest,
        options: ProxyOptions,
    ) -> Result<BlockingProxyHandle> {
        self.inner.ensure_auth()?;
        let req = self.inner.apply_metadata(req, &options.metadata);
        req.validate()?;
        let mut builder = self.inner.request(Method::POST, "/llm/proxy")?.json(&req);
        let accept = match options.stream_format {
            StreamFormat::Ndjson => "application/x-ndjson",
            StreamFormat::Sse => "text/event-stream",
        };
        builder = self.inner.with_headers(
            builder,
            options.request_id.as_deref(),
            &options.headers,
            Some(accept),
        )?;
        builder = self.inner.with_timeout(builder, options.timeout, false);
        let retry = options
            .retry
            .clone()
            .unwrap_or_else(|| self.inner.retry.clone());
        let mut ctx = self.inner.make_context(
            &Method::POST,
            "/llm/proxy",
            req.provider.clone(),
            Some(req.model.clone()),
            options.request_id.clone(),
        );
        let stream_start = Instant::now();
        let resp = self
            .inner
            .send_with_retry(builder, Method::POST, retry, ctx.clone())?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);
        ctx = ctx.with_request_id(request_id.clone());
        let stream_telemetry = self.inner.telemetry.stream_state(ctx, Some(stream_start));
        Ok(BlockingProxyHandle::new(
            resp,
            request_id,
            options.stream_format,
            stream_telemetry,
        ))
    }

    /// Convenience helper to stream text deltas directly (blocking).
    #[cfg(feature = "streaming")]
    pub fn proxy_stream_deltas(
        &self,
        req: ProxyRequest,
        options: ProxyOptions,
    ) -> Result<Box<dyn Iterator<Item = Result<String>>>> {
        let stream = self.proxy_stream(req, options)?;
        Ok(Box::new(
            ChatStreamAdapter::<crate::BlockingProxyHandle>::new(stream).into_iter(),
        ))
    }

    pub fn proxy(&self, req: ProxyRequest, options: ProxyOptions) -> Result<ProxyResponse> {
        self.inner.ensure_auth()?;
        let req = self.inner.apply_metadata(req, &options.metadata);
        req.validate()?;
        let mut builder = self.inner.request(Method::POST, "/llm/proxy")?.json(&req);
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
            "/llm/proxy",
            req.provider.clone(),
            Some(req.model.clone()),
            options.request_id.clone(),
        );
        let resp = self
            .inner
            .send_with_retry(builder, Method::POST, retry, ctx)?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        let bytes = resp
            .bytes()
            .map_err(|err| self.inner.to_transport_error(err, None))?;
        let mut payload: ProxyResponse =
            serde_json::from_slice(&bytes).map_err(Error::Serialization)?;
        payload.request_id = request_id;
        if self.inner.telemetry.usage_enabled() {
            let ctx = RequestContext::new(Method::POST.as_str(), "/llm/proxy")
                .with_provider(Some(payload.provider.clone()))
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
        if self
            .api_key
            .as_ref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
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

    fn apply_metadata(&self, mut req: ProxyRequest, metadata: &Option<HeaderList>) -> ProxyRequest {
        if let Some(default_meta) = &self.default_metadata {
            let mut map = req.metadata.unwrap_or_default();
            for entry in default_meta.iter() {
                if entry.is_valid() {
                    map.entry(entry.key.clone())
                        .or_insert_with(|| entry.value.clone());
                }
            }
            req.metadata = Some(map);
        }
        if let Some(meta) = metadata {
            let mut map = req.metadata.unwrap_or_default();
            for entry in meta.iter() {
                if entry.is_valid() {
                    map.insert(entry.key.clone(), entry.value.clone());
                }
            }
            req.metadata = Some(map);
        }
        req
    }

    fn make_context(
        &self,
        method: &Method,
        path: &str,
        provider: Option<Provider>,
        model: Option<Model>,
        request_id: Option<String>,
    ) -> RequestContext {
        RequestContext::new(method.as_str(), path)
            .with_provider(provider)
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
            builder = builder.header(API_KEY_HEADER, key);
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
    ) -> Result<Response> {
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
                    let body = resp.text().unwrap_or_default();
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
        let kind = if err.is_timeout() {
            TransportErrorKind::Timeout
        } else if err.is_connect() {
            TransportErrorKind::Connect
        } else if err.is_request() {
            TransportErrorKind::Request
        } else {
            TransportErrorKind::Other
        };

        TransportError {
            kind,
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

#[derive(Default)]
struct RetryState {
    attempts: u32,
    last_status: Option<u16>,
    last_error: Option<String>,
}

impl RetryState {
    fn new() -> Self {
        Self {
            attempts: 0,
            last_status: None,
            last_error: None,
        }
    }

    fn record_attempt(&mut self, attempt: u32) {
        self.attempts = attempt;
    }

    fn record_status(&mut self, status: StatusCode) {
        self.last_status = Some(status.as_u16());
    }

    fn record_error(&mut self, err: &reqwest::Error) {
        self.last_error = Some(err.to_string());
    }

    fn metadata(&self) -> Option<RetryMetadata> {
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

#[derive(serde::Deserialize)]
struct APIKeyResponse {
    #[serde(rename = "api_key")]
    api_key: APIKey,
}

#[cfg(feature = "streaming")]
#[derive(Clone)]
struct RawEvent {
    event: String,
    data: String,
}

#[cfg(feature = "streaming")]
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

#[cfg(feature = "streaming")]
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

#[cfg(feature = "streaming")]
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

#[cfg(feature = "streaming")]
fn map_event(raw: RawEvent, request_id: Option<String>) -> Option<StreamEvent> {
    let payload = serde_json::from_str::<serde_json::Value>(raw.data.as_str()).ok();
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
    }

    Some(event)
}
