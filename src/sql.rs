//! SQL validation API client.

use std::sync::Arc;

use reqwest::Method;

use crate::{
    client::ClientInner,
    errors::{Error, Result, ValidationError},
    generated::{SqlValidateRequest, SqlValidateResponse},
    http::HeaderList,
};

/// Client for SQL validation operations.
#[derive(Clone)]
pub struct SqlClient {
    pub(crate) inner: Arc<ClientInner>,
}

impl SqlClient {
    /// Validate a SQL query against a policy or profile.
    pub async fn validate(&self, req: SqlValidateRequest) -> Result<SqlValidateResponse> {
        if req.sql.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("sql is required").with_field("sql"),
            ));
        }

        let path = "/sql/validate";
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
        let resp: SqlValidateResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }
}
