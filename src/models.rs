//! Models catalog API client.
//!
//! Lists active models with rich metadata for building model selectors.

use std::sync::Arc;

use reqwest::Method;
use serde::Deserialize;

use crate::{
    client::ClientInner,
    errors::Result,
    generated::{ModelCapability, ProviderId},
    http::HeaderList,
};

/// CatalogModel is the model catalog entry returned by GET /models.
pub type CatalogModel = crate::generated::Model;

#[derive(Deserialize)]
struct ModelsListResponse {
    models: Vec<CatalogModel>,
}

/// Client for model catalog operations.
#[derive(Clone)]
pub struct ModelsClient {
    pub(crate) inner: Arc<ClientInner>,
}

impl ModelsClient {
    /// List active models with rich metadata.
    pub async fn list(
        &self,
        provider: Option<ProviderId>,
        capability: Option<ModelCapability>,
    ) -> Result<Vec<CatalogModel>> {
        let mut path = String::from("/models");
        let mut qs = Vec::<String>::new();
        if let Some(p) = provider {
            qs.push(format!("provider={p}"));
        }
        if let Some(c) = capability {
            qs.push(format!("capability={c}"));
        }
        if !qs.is_empty() {
            path.push('?');
            path.push_str(&qs.join("&"));
        }

        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp: ModelsListResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.models)
    }
}
