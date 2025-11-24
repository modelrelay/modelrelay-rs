use std::sync::Arc;

use reqwest::{
    Method,
    header::{ACCEPT, HeaderMap, HeaderName, HeaderValue},
};
use serde::de::DeserializeOwned;

use crate::{
    API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, REQUEST_ID_HEADER,
    errors::{APIError, Error, Result},
    types::{
        APIKey, APIKeyCreateRequest, FrontendToken, FrontendTokenRequest, ProxyRequest,
        ProxyResponse,
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
    pub http_client: Option<reqwest::Client>,
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
}

impl Client {
    pub fn new(cfg: Config) -> Result<Self> {
        let base = cfg
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let base_url = reqwest::Url::parse(&base)
            .map_err(|err| Error::Config(format!("invalid base url: {err}")))?;

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
            None => reqwest::Client::builder().build().map_err(Error::Http)?,
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

#[derive(Clone, Default)]
pub struct ProxyOptions {
    pub request_id: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl ProxyOptions {
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }
}

#[derive(Clone)]
pub struct LLMClient {
    inner: Arc<ClientInner>,
}

impl LLMClient {
    pub async fn proxy(&self, req: ProxyRequest, options: ProxyOptions) -> Result<ProxyResponse> {
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

        let resp = builder.send().await?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        if !resp.status().is_success() {
            return Err(parse_api_error(resp).await);
        }

        let mut payload: ProxyResponse = resp.json().await?;
        payload.request_id = request_id;
        Ok(payload)
    }

    #[cfg(feature = "streaming")]
    pub async fn proxy_stream(
        &self,
        req: ProxyRequest,
        options: ProxyOptions,
    ) -> Result<StreamHandle> {
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
            Some("text/event-stream"),
        )?;

        let resp = builder.send().await?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        if !resp.status().is_success() {
            return Err(parse_api_error(resp).await);
        }

        Ok(StreamHandle::new(resp, request_id))
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
        builder = self
            .inner
            .with_headers(builder, None, &[], Some("application/json"))?;

        self.inner.execute_json(builder).await
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
            &[],
            Some("application/json"),
        )?;
        let payload: APIKeysResponse = self.inner.execute_json(builder).await?;
        Ok(payload.api_keys)
    }

    pub async fn create(&self, req: APIKeyCreateRequest) -> Result<APIKey> {
        if req.label.trim().is_empty() {
            return Err(Error::Config("label is required".into()));
        }
        let mut builder = self.inner.request(Method::POST, "/api-keys")?.json(&req);
        builder = self
            .inner
            .with_headers(builder, None, &[], Some("application/json"))?;
        let payload: APIKeyResponse = self.inner.execute_json(builder).await?;
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
            &[],
            Some("application/json"),
        )?;
        let resp = builder.send().await?;
        if !resp.status().is_success() {
            return Err(parse_api_error(resp).await);
        }
        Ok(())
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
        headers: &[(String, String)],
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

    async fn execute_json<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<T> {
        let resp = builder.send().await?;
        if !resp.status().is_success() {
            return Err(parse_api_error(resp).await);
        }
        let parsed = resp.json::<T>().await?;
        Ok(parsed)
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

pub(crate) fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(REQUEST_ID_HEADER) {
        if let Ok(s) = value.to_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Some(value) = headers.get("X-Request-Id") {
        if let Ok(s) = value.to_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

async fn parse_api_error(resp: reqwest::Response) -> Error {
    let status = resp.status();
    let headers = resp.headers().clone();
    let request_id = request_id_from_headers(&headers);
    let status_code = status.as_u16();
    let status_text = status
        .canonical_reason()
        .unwrap_or("request failed")
        .to_string();

    let body = resp.text().await.unwrap_or_default();
    if body.is_empty() {
        return APIError {
            status: status_code,
            code: None,
            message: status_text,
            request_id,
            fields: Vec::new(),
        }
        .into();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(err_obj) = value.get("error").and_then(|v| v.as_object()) {
            let code = err_obj
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let message = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| status_text.clone());
            let fields = err_obj
                .get("fields")
                .and_then(|v| {
                    serde_json::from_value::<Vec<crate::errors::FieldError>>(v.clone()).ok()
                })
                .unwrap_or_default();
            let status_override = err_obj
                .get("status")
                .and_then(|v| v.as_u64())
                .map(|v| v as u16)
                .unwrap_or(status_code);
            return APIError {
                status: status_override,
                code,
                message,
                request_id: value
                    .get("request_id")
                    .or_else(|| value.get("requestId"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or(request_id),
                fields,
            }
            .into();
        }

        if let Some(message) = value.get("message").and_then(|v| v.as_str()) {
            let code = value
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let fields = value
                .get("fields")
                .and_then(|v| {
                    serde_json::from_value::<Vec<crate::errors::FieldError>>(v.clone()).ok()
                })
                .unwrap_or_default();
            return APIError {
                status: status_code,
                code,
                message: message.to_string(),
                request_id,
                fields,
            }
            .into();
        }
    }

    APIError {
        status: status_code,
        code: None,
        message: body,
        request_id,
        fields: Vec::new(),
    }
    .into()
}
