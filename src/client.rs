use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{
    Method, StatusCode,
    header::{ACCEPT, HeaderName, HeaderValue},
};
use serde::de::DeserializeOwned;
use tokio::time::sleep;

use crate::{
    API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_REQUEST_TIMEOUT, REQUEST_ID_HEADER,
    errors::{Error, Result, RetryMetadata, TransportError, TransportErrorKind},
    http::{HeaderList, ProxyOptions, RetryConfig, parse_api_error_parts, request_id_from_headers},
    telemetry::{HttpRequestMetrics, RequestContext, Telemetry, TokenUsageMetrics},
    types::{
        APIKey, APIKeyCreateRequest, FrontendToken, FrontendTokenRequest, Model, Provider,
        ProxyRequest, ProxyResponse,
    },
};

#[cfg(all(feature = "client", feature = "streaming"))]
use crate::sse::StreamHandle;

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub client_header: Option<String>,
    /// Environment preset (defaults to production). `base_url` takes precedence when set.
    pub environment: Option<crate::Environment>,
    pub http_client: Option<reqwest::Client>,
    /// Override the connect timeout (defaults to 5s).
    pub connect_timeout: Option<Duration>,
    /// Override the request timeout (defaults to 60s).
    pub timeout: Option<Duration>,
    /// Retry/backoff policy (defaults to 3 attempts, exponential backoff + jitter).
    pub retry: Option<RetryConfig>,
    /// Default extra headers applied to all requests.
    pub default_headers: Option<crate::http::HeaderList>,
    /// Default metadata applied to all proxy requests.
    pub default_metadata: Option<crate::http::HeaderList>,
    /// Optional metrics callbacks (HTTP latency, first-token latency, token usage).
    pub metrics: Option<crate::telemetry::MetricsCallbacks>,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    base_url: reqwest::Url,
    api_key: Option<String>,
    access_token: Option<String>,
    client_header: Option<String>,
    http: reqwest::Client,
    request_timeout: Duration,
    retry: RetryConfig,
    default_headers: Option<crate::http::HeaderList>,
    default_metadata: Option<crate::http::HeaderList>,
    telemetry: Telemetry,
}

impl Client {
    pub fn new(cfg: Config) -> Result<Self> {
        let base_source = cfg
            .base_url
            .clone()
            .or_else(|| cfg.environment.map(|env| env.base_url().to_string()))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let base = base_source.trim_end_matches('/').to_string();
        let base_url = reqwest::Url::parse(&base)
            .map_err(|err| Error::Config(format!("invalid base url: {err}")))?;

        let connect_timeout = cfg.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT);
        let request_timeout = cfg.timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT);
        let retry = cfg.retry.unwrap_or_default();

        if cfg
            .api_key
            .as_ref()
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
            && cfg
                .access_token
                .as_ref()
                .map(|v| v.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(Error::Config(
                "api key or access token is required".to_string(),
            ));
        }

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

    pub fn llm(&self) -> LLMClient {
        LLMClient {
            inner: self.inner.clone(),
        }
    }

    pub fn auth(&self) -> AuthClient {
        AuthClient {
            inner: self.inner.clone(),
        }
    }

    pub fn api_keys(&self) -> ApiKeysClient {
        ApiKeysClient {
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
            .map_err(|err| Error::Config(format!("invalid header name: {err}")))?;
        let val = HeaderValue::from_str(entry.value.trim())
            .map_err(|err| Error::Config(format!("invalid header value: {err}")))?;
        builder = builder.header(name, val);
    }
    Ok(builder)
}

#[derive(Clone)]
pub struct LLMClient {
    inner: Arc<ClientInner>,
}

impl LLMClient {
    pub async fn proxy(&self, req: ProxyRequest, options: ProxyOptions) -> Result<ProxyResponse> {
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
            .send_with_retry(builder, Method::POST, retry, ctx)
            .await?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        let bytes = resp
            .bytes()
            .await
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

    #[cfg(feature = "streaming")]
    pub async fn proxy_stream(
        &self,
        req: ProxyRequest,
        options: ProxyOptions,
    ) -> Result<StreamHandle> {
        let req = self.inner.apply_metadata(req, &options.metadata);
        req.validate()?;
        let mut builder = self.inner.request(Method::POST, "/llm/proxy")?.json(&req);
        builder = self.inner.with_headers(
            builder,
            options.request_id.as_deref(),
            &options.headers,
            Some("text/event-stream"),
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
            .send_with_retry(builder, Method::POST, retry, ctx.clone())
            .await?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);
        ctx = ctx.with_request_id(request_id.clone());
        let stream_telemetry = self.inner.telemetry.stream_state(ctx, Some(stream_start));

        Ok(StreamHandle::new(resp, request_id, stream_telemetry))
    }
}

#[derive(Clone)]
pub struct AuthClient {
    inner: Arc<ClientInner>,
}

impl AuthClient {
    pub async fn frontend_token(&self, mut req: FrontendTokenRequest) -> Result<FrontendToken> {
        if req.user_id.trim().is_empty() {
            return Err(Error::Config("user_id is required".into()));
        }
        if req
            .publishable_key
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            req.publishable_key = self.inner.api_key.clone();
        }
        if req
            .publishable_key
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            return Err(Error::Config("publishable key is required".into()));
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
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }
}

#[derive(Clone)]
pub struct ApiKeysClient {
    inner: Arc<ClientInner>,
}

impl ApiKeysClient {
    pub async fn list(&self) -> Result<Vec<APIKey>> {
        let builder = self.inner.with_headers(
            self.inner.request(Method::GET, "/api-keys")?,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, "/api-keys", None, None, None);
        let payload: APIKeysResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(payload.api_keys)
    }

    pub async fn create(&self, req: APIKeyCreateRequest) -> Result<APIKey> {
        if req.label.trim().is_empty() {
            return Err(Error::Config("label is required".into()));
        }
        let mut builder = self.inner.request(Method::POST, "/api-keys")?.json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::POST, "/api-keys", None, None, None);
        let payload: APIKeyResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(payload.api_key)
    }

    pub async fn delete(&self, id: uuid::Uuid) -> Result<()> {
        if id.is_nil() {
            return Err(Error::Config("id is required".into()));
        }
        let path = format!("/api-keys/{id}");
        let builder = self.inner.with_headers(
            self.inner.request(Method::DELETE, &path)?,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::DELETE, &path, None, None, None);
        self.inner
            .send_with_retry(builder, Method::DELETE, self.inner.retry.clone(), ctx)
            .await
            .map(|_| ())
    }
}

impl ClientInner {
    fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            reqwest::Url::parse(path).map_err(|err| Error::Config(err.to_string()))?
        } else {
            self.base_url
                .join(path)
                .map_err(|err| Error::Config(format!("invalid path: {err}")))?
        };
        Ok(self.http.request(method, url))
    }

    fn with_headers(
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

    fn with_timeout(
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
            builder = builder.header(API_KEY_HEADER, key);
        }
        builder
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
    async fn execute_json<T: DeserializeOwned>(
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

    async fn send_with_retry(
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
            let attempt_builder = builder
                .try_clone()
                .ok_or_else(|| Error::Config("request body is not cloneable for retry".into()))?;
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
                    let body = resp.text().await.unwrap_or_default();
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
struct APIKeysResponse {
    #[serde(rename = "api_keys")]
    api_keys: Vec<APIKey>,
}

#[derive(serde::Deserialize)]
struct APIKeyResponse {
    #[serde(rename = "api_key")]
    api_key: APIKey,
}
