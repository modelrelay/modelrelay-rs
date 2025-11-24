use std::sync::Arc;

use reqwest::{
    Method, Url,
    blocking::{Client as HttpClient, RequestBuilder, Response},
    header::{ACCEPT, HeaderName, HeaderValue},
};
use serde::de::DeserializeOwned;

use crate::{
    API_KEY_HEADER, DEFAULT_BASE_URL, DEFAULT_CLIENT_HEADER, REQUEST_ID_HEADER,
    errors::{Error, Result},
    http::{ProxyOptions, parse_api_error_parts, request_id_from_headers},
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
            None => HttpClient::builder().build().map_err(Error::Http)?,
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

        let resp = builder.send()?;
        let request_id = request_id_from_headers(resp.headers()).or(options.request_id);

        if !resp.status().is_success() {
            return Err(parse_blocking_api_error(resp));
        }

        let mut payload: ProxyResponse = resp.json()?;
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

        self.inner.execute_json(builder)
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
        let payload: APIKeysResponse = self.inner.execute_json(builder)?;
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
        let payload: APIKeyResponse = self.inner.execute_json(builder)?;
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
        let resp = builder.send()?;
        if !resp.status().is_success() {
            return Err(parse_blocking_api_error(resp));
        }
        Ok(())
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

    fn execute_json<T: DeserializeOwned>(&self, builder: RequestBuilder) -> Result<T> {
        let resp = builder.send()?;
        if !resp.status().is_success() {
            return Err(parse_blocking_api_error(resp));
        }
        let parsed = resp.json::<T>()?;
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

fn parse_blocking_api_error(resp: Response) -> Error {
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.text().unwrap_or_default();
    parse_api_error_parts(status, &headers, body)
}
