use reqwest::{header::ACCEPT, Method};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    client::ClientInner,
    errors::{APIError, Error, Result, WorkflowValidationError},
    http::HeaderList,
    workflow::{PlanHash, WorkflowSpecV0},
};

#[derive(Clone)]
pub struct WorkflowsClient {
    pub(crate) inner: std::sync::Arc<ClientInner>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowsCompileResponseV0 {
    pub plan_json: Value,
    pub plan_hash: PlanHash,
}

#[derive(Debug, Clone)]
pub enum WorkflowsCompileResultV0 {
    Ok(WorkflowsCompileResponseV0),
    ValidationError(WorkflowValidationError),
    InternalError(APIError),
}

impl WorkflowsClient {
    pub async fn compile_v0(&self, spec: WorkflowSpecV0) -> Result<WorkflowsCompileResultV0> {
        self.inner.ensure_auth()?;

        let path = "/workflows/compile";
        let mut builder = self.inner.request(Method::POST, path)?;
        // Request body is the workflow spec itself.
        builder = builder.json(&spec);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = builder.header(ACCEPT, "application/json");
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, path, None, None);

        match self
            .inner
            .execute_json::<WorkflowsCompileResponseV0>(builder, Method::POST, None, ctx)
            .await
        {
            Ok(out) => Ok(WorkflowsCompileResultV0::Ok(out)),
            Err(Error::WorkflowValidation(verr)) => {
                Ok(WorkflowsCompileResultV0::ValidationError(verr))
            }
            Err(Error::Api(api_err)) => Ok(WorkflowsCompileResultV0::InternalError(api_err)),
            Err(other) => Err(other),
        }
    }
}
