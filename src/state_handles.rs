//! State handle API client.

use std::sync::Arc;

use reqwest::Method;

use crate::{
    client::ClientInner,
    errors::{Result, ValidationError},
    generated::{StateHandleCreateRequest, StateHandleListResponse, StateHandleResponse},
    http::HeaderList,
    Error,
};

/// Maximum allowed TTL for a state handle (1 year in seconds).
pub const MAX_STATE_HANDLE_TTL_SECONDS: u64 = 31536000;

/// Client for state handle operations.
#[derive(Clone)]
pub struct StateHandlesClient {
    pub(crate) inner: Arc<ClientInner>,
}

/// Options for listing state handles.
#[derive(Debug, Default, Clone)]
pub struct ListStateHandlesOptions {
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

impl StateHandlesClient {
    /// Create a state handle for persistent /responses tool state.
    pub async fn create(&self, req: StateHandleCreateRequest) -> Result<StateHandleResponse> {
        // Note: ttl_seconds is NonZero<u64> due to OpenAPI minimum: 1, so no need to check for <= 0
        if let Some(ttl_seconds) = req.ttl_seconds {
            if ttl_seconds.get() > MAX_STATE_HANDLE_TTL_SECONDS {
                return Err(Error::Validation(
                    ValidationError::new("ttl_seconds exceeds maximum (1 year)")
                        .with_field("ttl_seconds"),
                ));
            }
        }

        let path = "/state-handles";
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
        let resp: StateHandleResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }

    /// List state handles with pagination.
    pub async fn list(&self, opts: ListStateHandlesOptions) -> Result<StateHandleListResponse> {
        if let Some(limit) = opts.limit {
            if limit <= 0 || limit > 100 {
                return Err(Error::Validation(
                    ValidationError::new("limit must be between 1 and 100").with_field("limit"),
                ));
            }
        }
        if let Some(offset) = opts.offset {
            if offset < 0 {
                return Err(Error::Validation(
                    ValidationError::new("offset must be non-negative").with_field("offset"),
                ));
            }
        }

        let mut path = "/state-handles".to_string();
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(limit) = opts.limit {
            params.push(("limit", limit.to_string()));
        }
        if let Some(offset) = opts.offset {
            params.push(("offset", offset.to_string()));
        }
        if !params.is_empty() {
            let encoded: String = params
                .iter()
                .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            path = format!("{}?{}", path, encoded);
        }

        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp: StateHandleListResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp)
    }

    /// Delete a state handle by ID.
    pub async fn delete(&self, state_id: &str) -> Result<()> {
        if state_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("state_id is required").with_field("state_id"),
            ));
        }

        let path = format!("/state-handles/{}", state_id);
        let builder = self.inner.request(Method::DELETE, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let retry_cfg = self.inner.retry.clone();
        let ctx = self.inner.make_context(&Method::DELETE, &path, None, None);
        let _ = self
            .inner
            .send_with_retry(builder, Method::DELETE, retry_cfg, ctx)
            .await?;
        Ok(())
    }
}
