use std::{sync::Arc, thread, time::Duration};

use reqwest::{
    Method, StatusCode, Url,
    blocking::{Client as HttpClient, RequestBuilder, Response},
    header::{ACCEPT, HeaderName, HeaderValue},
};
use serde::de::DeserializeOwned;

use crate::{
    API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_REQUEST_TIMEOUT, REQUEST_ID_HEADER,
    errors::{Error, Result, RetryMetadata, TransportError, TransportErrorKind},
    http::{ProxyOptions, RetryConfig, parse_api_error_parts, request_id_from_headers},
    types::{
        APIKey, APIKeyCreateRequest, FrontendToken, FrontendTokenRequest, ProxyRequest,
        ProxyResponse,
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
}

impl BlockingClient {
    pub fn new(cfg: BlockingConfig) -> Result<Self> {
        let base = cfg
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let base_url =
            Url::parse(&base).map_err(|err| Error::Config(format!("invalid base url: {err}")))?;

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

    pub fn api_keys(&self) -> BlockingApiKeysClient {
        BlockingApiKeysClient {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Clone)]
pub struct BlockingLLMClient {
    inner: Arc<ClientInner>,
}

impl BlockingLLMClient {
    pub fn proxy(&self, req: ProxyRequest, options: ProxyOptions) -> Result<ProxyResponse> {
        if req.model.trim().is_empty() {
            return Err(Error::Config("model is required".into()));
        }
        if req.messages.is_empty() {
            return Err(Error::Config("at least one message is required".into()));
        }

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

        let resp = self.inner.send_with_retry(builder, Method::POST, retry)?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        let bytes = resp
            .bytes()
            .map_err(|err| self.inner.to_transport_error(err, None))?;
        let mut payload: ProxyResponse =
            serde_json::from_slice(&bytes).map_err(Error::Serialization)?;
        payload.request_id = request_id;
        Ok(payload)
    }
}

#[derive(Clone)]
pub struct BlockingAuthClient {
    inner: Arc<ClientInner>,
}

impl BlockingAuthClient {
    pub fn frontend_token(&self, mut req: FrontendTokenRequest) -> Result<FrontendToken> {
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
        builder = self
            .inner
            .with_headers(builder, None, &[], Some("application/json"))?;

        builder = self.inner.with_timeout(builder, None, true);
        self.inner.execute_json(builder, Method::POST, None)
    }
}

#[derive(Clone)]
pub struct BlockingApiKeysClient {
    inner: Arc<ClientInner>,
}

impl BlockingApiKeysClient {
    pub fn list(&self) -> Result<Vec<APIKey>> {
        let builder = self.inner.with_headers(
            self.inner.request(Method::GET, "/api-keys")?,
            None,
            &[],
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let payload: APIKeysResponse = self.inner.execute_json(builder, Method::GET, None)?;
        Ok(payload.api_keys)
    }

    pub fn create(&self, req: APIKeyCreateRequest) -> Result<APIKey> {
        if req.label.trim().is_empty() {
            return Err(Error::Config("label is required".into()));
        }
        let mut builder = self.inner.request(Method::POST, "/api-keys")?.json(&req);
        builder = self
            .inner
            .with_headers(builder, None, &[], Some("application/json"))?;
        let builder = self.inner.with_timeout(builder, None, true);
        let payload: APIKeyResponse = self.inner.execute_json(builder, Method::POST, None)?;
        Ok(payload.api_key)
    }

    pub fn delete(&self, id: uuid::Uuid) -> Result<()> {
        if id.is_nil() {
            return Err(Error::Config("id is required".into()));
        }
        let path = format!("/api-keys/{id}");
        let builder = self.inner.with_headers(
            self.inner.request(Method::DELETE, &path)?,
            None,
            &[],
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        self.inner
            .send_with_retry(builder, Method::DELETE, self.inner.retry.clone())
            .map(|_| ())
    }
}

impl ClientInner {
    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            Url::parse(path).map_err(|err| Error::Config(err.to_string()))?
        } else {
            self.base_url
                .join(path)
                .map_err(|err| Error::Config(format!("invalid path: {err}")))?
        };
        Ok(self.http.request(method, url))
    }

    fn with_headers(
        &self,
        mut builder: RequestBuilder,
        request_id: Option<&str>,
        headers: &[(String, String)],
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

        for (key, value) in headers {
            if key.trim().is_empty() || value.trim().is_empty() {
                continue;
            }
            let name = HeaderName::from_bytes(key.trim().as_bytes())
                .map_err(|err| Error::Config(format!("invalid header name: {err}")))?;
            let val = HeaderValue::from_str(value.trim())
                .map_err(|err| Error::Config(format!("invalid header value: {err}")))?;
            builder = builder.header(name, val);
        }

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
    ) -> Result<T> {
        let retry_cfg = retry.unwrap_or_else(|| self.retry.clone());
        let resp = self.send_with_retry(builder, method, retry_cfg)?;
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
    ) -> Result<Response> {
        let max_attempts = retry.max_attempts.max(1);
        let mut state = RetryState::new();

        for attempt in 1..=max_attempts {
            let attempt_builder = builder
                .try_clone()
                .ok_or_else(|| Error::Config("request body is not cloneable for retry".into()))?;
            let result = attempt_builder.send();

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    }
                    state.record_attempt(attempt);
                    state.record_status(status);

                    let should_retry = retry.should_retry_status(&method, status);
                    if should_retry && attempt < max_attempts {
                        thread::sleep(retry.backoff_delay(attempt));
                        continue;
                    }

                    let headers = resp.headers().clone();
                    let body = resp.text().unwrap_or_default();
                    return Err(parse_api_error_parts(
                        status,
                        &headers,
                        body,
                        state.metadata(),
                    ));
                }
                Err(err) => {
                    state.record_attempt(attempt);
                    state.record_error(&err);
                    let should_retry = retry.should_retry_error(&method, &err);
                    if should_retry && attempt < max_attempts {
                        thread::sleep(retry.backoff_delay(attempt));
                        continue;
                    }

                    return Err(self.to_transport_error(err, state.metadata()));
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
