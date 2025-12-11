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
    blocking::{Client as HttpClient, RequestBuilder, Response},
    header::{HeaderName, HeaderValue, ACCEPT},
    Method, Url,
};
use serde::de::DeserializeOwned;

#[cfg(all(feature = "blocking", feature = "streaming"))]
use crate::chat::ChatStreamAdapter;
use crate::chat::CustomerProxyRequestBody;
use crate::core::RetryState;
#[cfg(feature = "streaming")]
use crate::core::{consume_ndjson_buffer, map_event};
#[cfg(feature = "streaming")]
use crate::telemetry::StreamTelemetry;
#[cfg(feature = "streaming")]
use crate::types::{StopReason, StreamEvent, StreamEventKind, Usage};
use crate::{
    errors::{Error, Result, RetryMetadata, TransportError, TransportErrorKind, ValidationError},
    http::{parse_api_error_parts, request_id_from_headers, HeaderList, ProxyOptions, RetryConfig},
    telemetry::{HttpRequestMetrics, RequestContext, Telemetry, TokenUsageMetrics},
    types::{
        FrontendToken, FrontendTokenAutoProvisionRequest, FrontendTokenRequest, Model,
        ProxyRequest, ProxyResponse,
    },
    API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_REQUEST_TIMEOUT, REQUEST_ID_HEADER,
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
    telemetry: Telemetry,
}

/// Blocking NDJSON streaming handle for LLM proxy.
#[cfg(feature = "streaming")]
pub struct BlockingProxyHandle {
    request_id: Option<String>,
    response: Option<Response>,
    buffer: String,
    pending: VecDeque<StreamEvent>,
    finished: bool,
    telemetry: Option<StreamTelemetry>,
}

#[cfg(feature = "streaming")]
impl BlockingProxyHandle {
    fn new(
        response: Response,
        request_id: Option<String>,
        telemetry: Option<StreamTelemetry>,
    ) -> Self {
        Self {
            request_id,
            response: Some(response),
            buffer: String::new(),
            pending: VecDeque::new(),
            finished: false,
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

    /// Pull next streaming event.
    #[allow(clippy::should_implement_trait)]
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
                let (events, _) = consume_ndjson_buffer(&self.buffer);
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
            let (events, remainder) = consume_ndjson_buffer(&self.buffer);
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
    /// Exchange a publishable key for a short-lived bearer token for an existing customer.
    pub fn frontend_token(&self, req: FrontendTokenRequest) -> Result<FrontendToken> {
        if req.customer_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("customer_id is required").with_field("customer_id"),
            ));
        }
        if req.publishable_key.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("publishable key is required").with_field("publishable_key"),
            ));
        }

        self.send_frontend_token_request(&req)
    }

    /// Exchange a publishable key for a frontend token, creating the customer if needed.
    /// The customer will be auto-provisioned on the project's free tier.
    pub fn frontend_token_auto_provision(
        &self,
        req: FrontendTokenAutoProvisionRequest,
    ) -> Result<FrontendToken> {
        if req.customer_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("customer_id is required").with_field("customer_id"),
            ));
        }
        if req.publishable_key.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("publishable key is required").with_field("publishable_key"),
            ));
        }
        if req.email.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("email is required for auto-provisioning").with_field("email"),
            ));
        }

        self.send_frontend_token_request(&req)
    }

    fn send_frontend_token_request<T: serde::Serialize>(&self, req: &T) -> Result<FrontendToken> {
        let mut builder = self
            .inner
            .request(Method::POST, "/auth/frontend-token")?
            .json(req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;

        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::POST, "/auth/frontend-token", None, None);
        self.inner.execute_json(builder, Method::POST, None, ctx)
    }
}

#[derive(Clone)]
pub struct BlockingLLMClient {
    inner: Arc<ClientInner>,
}

impl BlockingLLMClient {
    /// Streaming handle over NDJSON chat events (blocking client).
    #[cfg(feature = "streaming")]
    pub fn proxy_stream(
        &self,
        req: ProxyRequest,
        options: ProxyOptions,
    ) -> Result<BlockingProxyHandle> {
        self.inner.ensure_auth()?;
        req.validate()?;
        let mut builder = self.inner.request(Method::POST, "/llm/proxy")?.json(&req);
        builder = self.inner.with_headers(
            builder,
            options.request_id.as_deref(),
            &options.headers,
            Some("application/x-ndjson"),
        )?;
        builder = self.inner.with_timeout(builder, options.timeout, false);
        let retry = options
            .retry
            .clone()
            .unwrap_or_else(|| self.inner.retry.clone());
        let mut ctx = self.inner.make_context(
            &Method::POST,
            "/llm/proxy",
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
        Ok(BlockingProxyHandle::new(resp, request_id, stream_telemetry))
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

    /// Execute a customer-attributed proxy request (non-streaming).
    ///
    /// The `customer_id` is sent via the `X-ModelRelay-Customer-Id` header.
    pub fn proxy_customer(
        &self,
        customer_id: &str,
        body: CustomerProxyRequestBody,
        options: ProxyOptions,
    ) -> Result<ProxyResponse> {
        if customer_id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "customer ID is required",
            )));
        }
        self.inner.ensure_auth()?;
        let mut builder = self.inner.request(Method::POST, "/llm/proxy")?.json(&body);
        // Add customer ID header
        let options = options.with_header(crate::chat::CUSTOMER_ID_HEADER, customer_id);
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
            None,
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

    /// Execute a customer-attributed proxy request (streaming).
    ///
    /// The `customer_id` is sent via the `X-ModelRelay-Customer-Id` header.
    #[cfg(feature = "streaming")]
    pub fn proxy_customer_stream(
        &self,
        customer_id: &str,
        body: CustomerProxyRequestBody,
        options: ProxyOptions,
    ) -> Result<BlockingProxyHandle> {
        if customer_id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "customer ID is required",
            )));
        }
        self.inner.ensure_auth()?;
        let mut builder = self.inner.request(Method::POST, "/llm/proxy")?.json(&body);
        // Add customer ID header
        let options = options.with_header(crate::chat::CUSTOMER_ID_HEADER, customer_id);
        builder = self.inner.with_headers(
            builder,
            options.request_id.as_deref(),
            &options.headers,
            Some("application/x-ndjson"),
        )?;
        builder = self.inner.with_timeout(builder, options.timeout, false);
        let retry = options
            .retry
            .clone()
            .unwrap_or_else(|| self.inner.retry.clone());
        let mut ctx = self.inner.make_context(
            &Method::POST,
            "/llm/proxy",
            None,
            options.request_id.clone(),
        );
        let stream_start = Instant::now();
        let resp = self
            .inner
            .send_with_retry(builder, Method::POST, retry, ctx.clone())?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);
        ctx = ctx.with_request_id(request_id.clone());
        let stream_telemetry = self.inner.telemetry.stream_state(ctx, Some(stream_start));
        Ok(BlockingProxyHandle::new(resp, request_id, stream_telemetry))
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

// RetryState, RawEvent, consume_ndjson_buffer, and map_event are now in core.rs
