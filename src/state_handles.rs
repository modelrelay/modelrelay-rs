//! State handle API client.

use std::sync::Arc;

use reqwest::Method;

use crate::{
    client::ClientInner,
    errors::{Result, ValidationError},
    generated::{StateHandleCreateRequest, StateHandleResponse},
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
}
