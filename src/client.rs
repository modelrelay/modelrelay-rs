use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{
    header::{HeaderName, HeaderValue, ACCEPT},
    Method,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use tokio::time::sleep;

use crate::core::RetryState;
use crate::{
    customers::CustomersClient,
    errors::{Error, Result, RetryMetadata, TransportError, TransportErrorKind, ValidationError},
    http::{
        parse_api_error_parts, request_id_from_headers, HeaderList, ResponseOptions, RetryConfig,
    },
    runs::RunsClient,
    telemetry::{HttpRequestMetrics, RequestContext, Telemetry, TokenUsageMetrics},
    tiers::TiersClient,
    types::{CustomerToken, CustomerTokenRequest, Model, Response, ResponseRequest},
    workflows::WorkflowsClient,
    ApiKey, API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_REQUEST_TIMEOUT, REQUEST_ID_HEADER,
};

#[cfg(feature = "streaming")]
use crate::ndjson::StreamHandle;

/// Configuration for the async ModelRelay client.
///
/// Use this to configure authentication, timeouts, retries, and other options.
/// For the blocking (synchronous) client, see `BlockingConfig` (requires the `blocking` feature).
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// Base URL for the ModelRelay API (defaults to `https://api.modelrelay.ai/api/v1`).
    pub base_url: Option<String>,
    /// API key for authentication (`mr_sk_*` for secret key, `mr_pk_*` for publishable key).
    pub api_key: Option<ApiKey>,
    /// Bearer token for authentication (alternative to API key).
    pub access_token: Option<String>,
    /// Custom client identifier sent in headers for debugging/analytics.
    pub client_header: Option<String>,
    /// Custom HTTP client instance (uses default if not provided).
    pub http_client: Option<reqwest::Client>,
    /// Override the connect timeout (defaults to 5s).
    pub connect_timeout: Option<Duration>,
    /// Override the request timeout (defaults to 60s).
    pub timeout: Option<Duration>,
    /// Retry/backoff policy (defaults to 3 attempts, exponential backoff + jitter).
    pub retry: Option<RetryConfig>,
    /// Default extra headers applied to all requests.
    pub default_headers: Option<crate::http::HeaderList>,
    /// Optional metrics callbacks (HTTP latency, first-token latency, token usage).
    pub metrics: Option<crate::telemetry::MetricsCallbacks>,
}

/// Async client for the ModelRelay API.
///
/// This is the primary client for making API requests. For synchronous contexts,
/// use `BlockingClient` instead (requires the `blocking` feature).
///
/// # Example
///
/// ```ignore
/// use modelrelay::{Client, Config, ResponseBuilder};
///
/// let client = Client::new(Config {
///     api_key: Some(modelrelay::ApiKey::parse("mr_sk_...")?),
///     ..Default::default()
/// })?;
///
/// let response = ResponseBuilder::new()
///     .model("gpt-4o-mini")
///     .user("Hello!")
///     .send(&client.responses())
///     .await?;
/// ```
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

pub(crate) struct ClientInner {
    pub(crate) base_url: reqwest::Url,
    pub(crate) api_key: Option<ApiKey>,
    pub(crate) access_token: Option<String>,
    pub(crate) client_header: Option<String>,
    pub(crate) http: reqwest::Client,
    pub(crate) request_timeout: Duration,
    pub(crate) retry: RetryConfig,
    pub(crate) default_headers: Option<crate::http::HeaderList>,
    pub(crate) telemetry: Telemetry,
}

impl Client {
    /// Creates a new client with the given configuration.
    ///
    /// **Note:** Either `api_key` or `access_token` must be provided. This is validated
    /// at construction time and will return an error if neither is set.
    ///
    /// For clearer intent, consider using [`Client::with_key`] or [`Client::with_token`]
    /// which make the authentication requirement explicit.
    pub fn new(cfg: Config) -> Result<Self> {
        // Validate auth is provided at construction time (not deferred to request time)
        let has_key = cfg.api_key.is_some();
        let has_token = cfg
            .access_token
            .as_ref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        if !has_key && !has_token {
            return Err(Error::Validation(crate::errors::ValidationError::new(
                "api key or access token is required",
            )));
        }

        let base_source = cfg
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        // Treat the base as a directory so relative joins keep the versioned prefix ("/api/v1/").
        let base = format!("{}/", base_source.trim_end_matches('/'));
        let base_url = reqwest::Url::parse(&base)
            .map_err(|err| Error::Validation(format!("invalid base url: {err}").into()))?;

        let connect_timeout = cfg.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT);
        let request_timeout = cfg.timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT);
        let retry = cfg.retry.unwrap_or_default();

        let http = match cfg.http_client {
            Some(client) => client,
            None => reqwest::Client::builder()
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

    /// Creates a new client authenticated with an API key.
    ///
    /// The key is required and must be non-empty. Use [`ClientBuilder`] for additional
    /// configuration options.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use modelrelay::Client;
    ///
    /// let client = Client::with_key(modelrelay::ApiKey::parse("mr_sk_...")?).build()?;
    /// # Ok::<(), modelrelay::Error>(())
    /// ```
    pub fn with_key(key: impl Into<ApiKey>) -> ClientBuilder {
        ClientBuilder::new().api_key(key)
    }

    /// Creates a new client authenticated with a bearer access token.
    ///
    /// The token is required and must be non-empty. Use [`ClientBuilder`] for additional
    /// configuration options.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use modelrelay::Client;
    ///
    /// let client = Client::with_token("eyJ...").build()?;
    /// # Ok::<(), modelrelay::Error>(())
    /// ```
    pub fn with_token(token: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new().access_token(token)
    }

    /// Returns a builder for more complex client configuration.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Returns the Responses client for `POST /responses` (streaming + non-streaming).
    pub fn responses(&self) -> ResponsesClient {
        ResponsesClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the auth client for customer token operations.
    pub fn auth(&self) -> AuthClient {
        AuthClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the customers client for customer management.
    ///
    /// Requires a secret key (`mr_sk_*`) for authentication.
    pub fn customers(&self) -> CustomersClient {
        CustomersClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the tiers client for tier operations.
    ///
    /// Requires a secret key (`mr_sk_*`) for authentication.
    pub fn tiers(&self) -> TiersClient {
        TiersClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the runs client for workflow runs (`/runs`).
    pub fn runs(&self) -> RunsClient {
        RunsClient {
            inner: self.inner.clone(),
        }
    }

    /// Returns the workflows client for compilation/validation (`/workflows`).
    pub fn workflows(&self) -> WorkflowsClient {
        WorkflowsClient {
            inner: self.inner.clone(),
        }
    }
}

fn apply_header_list(
    mut builder: reqwest::RequestBuilder,
    headers: &HeaderList,
) -> Result<reqwest::RequestBuilder> {
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

/// Async client for `POST /responses`.
///
/// Prefer using [`ResponseBuilder`] for ergonomic request construction.
///
/// [`ResponseBuilder`]: crate::ResponseBuilder
#[derive(Clone)]
pub struct ResponsesClient {
    inner: Arc<ClientInner>,
}

impl ResponsesClient {
    pub(crate) async fn create(
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
            .send_with_retry(builder, Method::POST, retry, ctx)
            .await?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        let bytes = resp
            .bytes()
            .await
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

    /// Create a structured response with schema inference + validation retries.
    ///
    /// This executes `POST /responses` and (optionally) retries by appending
    /// corrective messages when the model output cannot be decoded into `T`.
    pub async fn create_structured<T, H>(
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
                .send(self)
                .await
                .map_err(crate::structured::StructuredError::Sdk)?;
            match executor.process_response::<T>(&response, &inner.input)? {
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

    #[cfg(feature = "streaming")]
    pub(crate) async fn stream(
        &self,
        req: ResponseRequest,
        options: ResponseOptions,
    ) -> Result<StreamHandle> {
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
            Some("application/x-ndjson"),
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
            .send_with_retry(builder, Method::POST, retry, ctx.clone())
            .await?;

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_lowercase());
        let is_ndjson = content_type
            .as_deref()
            .map(|ct| {
                ct.starts_with("application/x-ndjson") || ct.starts_with("application/ndjson")
            })
            .unwrap_or(false);
        if !is_ndjson {
            return Err(Error::StreamContentType {
                expected: "application/x-ndjson",
                received: content_type.unwrap_or_else(|| "<missing>".to_string()),
                status: resp.status().as_u16(),
            });
        }

        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);
        ctx = ctx.with_request_id(request_id.clone());
        let stream_telemetry = self.inner.telemetry.stream_state(ctx, Some(stream_start));

        Ok(StreamHandle::new(
            resp,
            request_id,
            stream_telemetry,
            options.stream_timeouts,
            stream_start,
        ))
    }

    /// Convenience helper for the common "system + user -> assistant text" path.
    ///
    /// This is a thin wrapper around `ResponseBuilder` and `Response::text()`.
    /// Returns an `EmptyResponse` transport error if the response contains no
    /// assistant text output.
    pub async fn text(
        &self,
        model: impl Into<crate::types::Model>,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Result<String> {
        crate::responses::ResponseBuilder::text_prompt(system, user)
            .model(model)
            .send_text(self)
            .await
    }

    /// Convenience helper for customer-attributed requests where the backend selects the model.
    ///
    /// This sets the customer id header and omits `model` from the request body.
    pub async fn text_for_customer(
        &self,
        customer_id: impl Into<String>,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Result<String> {
        crate::responses::ResponseBuilder::text_prompt(system, user)
            .customer_id(customer_id)
            .send_text(self)
            .await
    }

    /// Convenience helper to stream text deltas directly.
    #[cfg(feature = "streaming")]
    pub async fn stream_text_deltas(
        &self,
        model: impl Into<crate::types::Model>,
        system: impl Into<String>,
        user: impl Into<String>,
    ) -> Result<std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<String>> + Send>>> {
        crate::responses::ResponseBuilder::text_prompt(system, user)
            .model(model)
            .stream_deltas(self)
            .await
    }
}

/// Async client for customer token operations.
#[derive(Clone)]
pub struct AuthClient {
    inner: Arc<ClientInner>,
}

impl AuthClient {
    /// Mint a customer-scoped bearer token (requires secret key auth).
    pub async fn customer_token(&self, req: CustomerTokenRequest) -> Result<CustomerToken> {
        if req.project_id.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("project_id is required").with_field("project_id"),
            ));
        }
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
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }
}

impl ClientInner {
    pub(crate) fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            reqwest::Url::parse(path).map_err(|err| Error::Validation(err.to_string().into()))?
        } else {
            let rel = path.trim_start_matches('/');
            self.base_url
                .join(rel)
                .map_err(|err| Error::Validation(format!("invalid path: {err}").into()))?
        };
        Ok(self.http.request(method, url))
    }

    pub(crate) fn with_headers(
        &self,
        mut builder: reqwest::RequestBuilder,
        request_id: Option<&str>,
        headers: &HeaderList,
        accept: Option<&str>,
    ) -> Result<reqwest::RequestBuilder> {
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

    pub(crate) fn with_timeout(
        &self,
        builder: reqwest::RequestBuilder,
        timeout: Option<Duration>,
        use_default: bool,
    ) -> reqwest::RequestBuilder {
        if let Some(duration) = timeout {
            builder.timeout(duration)
        } else if use_default {
            builder.timeout(self.request_timeout)
        } else {
            builder
        }
    }

    pub(crate) fn ensure_auth(&self) -> Result<()> {
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

    pub(crate) fn has_jwt_access_token(&self) -> bool {
        let Some(token) = self.access_token.as_deref() else {
            return false;
        };
        let t = token.trim();
        if t.is_empty() {
            return false;
        }
        // Treat API keys passed as bearer tokens as non-JWT for model validation.
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("mr_sk_") || lower.starts_with("mr_pk_") {
            return false;
        }
        // JWTs have 3 base64url segments separated by '.'.
        t.matches('.').count() >= 2
    }

    fn apply_auth(&self, mut builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
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

    pub(crate) fn make_context(
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
    pub(crate) async fn execute_json<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
        method: Method,
        retry: Option<RetryConfig>,
        ctx: RequestContext,
    ) -> Result<T> {
        let retry_cfg = retry.unwrap_or_else(|| self.retry.clone());
        let resp = self
            .send_with_retry(builder, method, retry_cfg, ctx)
            .await?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|err| self.to_transport_error(err, None))?;
        let parsed = serde_json::from_slice::<T>(&bytes).map_err(Error::Serialization)?;
        Ok(parsed)
    }

    pub(crate) async fn send_with_retry(
        &self,
        builder: reqwest::RequestBuilder,
        method: Method,
        retry: RetryConfig,
        ctx: RequestContext,
    ) -> Result<reqwest::Response> {
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
            let result = attempt_builder.send().await;

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
                        sleep(retry.backoff_delay(attempt)).await;
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
                    let body = match resp.text().await {
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
                        sleep(retry.backoff_delay(attempt)).await;
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

// RetryState is now in core.rs

/// Builder for constructing a [`Client`] with explicit configuration.
///
/// Use [`Client::with_key`], [`Client::with_token`], or [`Client::builder`] to create a builder.
///
/// # Examples
///
/// ```no_run
/// use modelrelay::Client;
///
/// // With API key
/// let client = Client::with_key(modelrelay::ApiKey::parse("mr_sk_...")?)
///     .base_url("https://custom.api.com")
///     .build()?;
///
/// // With access token
/// let client = Client::with_token("eyJ...")
///     .timeout(std::time::Duration::from_secs(30))
///     .build()?;
/// # Ok::<(), modelrelay::Error>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct ClientBuilder {
    config: Config,
}

impl ClientBuilder {
    /// Creates a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: Config::default(),
        }
    }

    /// Sets the API key for authentication.
    ///
    /// API keys are prefixed with `mr_sk_` (secret) or `mr_pk_` (publishable).
    pub fn api_key(mut self, key: impl Into<ApiKey>) -> Self {
        self.config.api_key = Some(key.into());
        self
    }

    /// Sets the bearer access token for authentication.
    pub fn access_token(mut self, token: impl Into<String>) -> Self {
        self.config.access_token = Some(token.into());
        self
    }

    /// Sets the API base URL (defaults to production).
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.config.base_url = Some(url.into());
        self
    }

    /// Sets a custom HTTP client.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.config.http_client = Some(client);
        self
    }

    /// Sets the X-ModelRelay-Client header for SDK identification.
    pub fn client_header(mut self, header: impl Into<String>) -> Self {
        self.config.client_header = Some(header.into());
        self
    }

    /// Sets the connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = Some(timeout);
        self
    }

    /// Sets the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    /// Sets the retry configuration.
    pub fn retry(mut self, retry: RetryConfig) -> Self {
        self.config.retry = Some(retry);
        self
    }

    /// Sets default headers applied to every request.
    pub fn default_headers(mut self, headers: crate::http::HeaderList) -> Self {
        self.config.default_headers = Some(headers);
        self
    }

    /// Sets metrics callbacks for observability.
    pub fn metrics(mut self, callbacks: crate::telemetry::MetricsCallbacks) -> Self {
        self.config.metrics = Some(callbacks);
        self
    }

    /// Builds the client, validating that authentication is configured.
    ///
    /// Returns an error if neither API key nor access token is set.
    pub fn build(self) -> Result<Client> {
        Client::new(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_new_requires_auth() {
        // Default config has no auth - should fail
        let result = Client::new(Config::default());
        match result {
            Err(Error::Validation(v)) => {
                assert!(v.to_string().contains("api key or access token"));
            }
            Err(e) => panic!("expected ValidationError, got {:?}", e),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn client_new_accepts_api_key() {
        let result = Client::new(Config {
            api_key: Some(ApiKey::parse("mr_sk_test").unwrap()),
            ..Config::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn client_new_accepts_access_token() {
        let result = Client::new(Config {
            access_token: Some("eyJ...".to_string()),
            ..Config::default()
        });
        assert!(result.is_ok());
    }

    #[test]
    fn api_key_parse_rejects_invalid() {
        assert!(ApiKey::parse("").is_err());
        assert!(ApiKey::parse("mr_zzz_123").is_err());
        assert!(ApiKey::parse("mr_sk_").is_err());
        assert!(ApiKey::parse("mr_pk_").is_err());
    }

    #[test]
    fn client_with_key_creates_builder() {
        let client = Client::with_key(ApiKey::parse("mr_sk_test").unwrap()).build();
        assert!(client.is_ok());
    }

    #[test]
    fn client_with_token_creates_builder() {
        let client = Client::with_token("eyJ...").build();
        assert!(client.is_ok());
    }
}
