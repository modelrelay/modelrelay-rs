use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    client::ClientInner,
    core::consume_ndjson_buffer,
    errors::{Error, Result, TransportError, ValidationError},
    generated::{RunsPendingToolsResponse, ToolCallId, ToolName},
    http::{request_id_from_headers, validate_ndjson_content_type, HeaderList},
    workflow::{
        NodeId, NodeResultV0, PlanHash, RequestId, RunCostSummaryV0, RunEventV0, RunId, RunStatusV0,
    },
    workflow_intent::WorkflowIntentSpec,
};

#[cfg(feature = "streaming")]
use crate::ndjson::classify_reqwest_error;
#[cfg(feature = "streaming")]
use futures_core::Stream;
#[cfg(feature = "streaming")]
use futures_util::{stream, StreamExt};

#[derive(Clone)]
pub struct RunsClient {
    pub(crate) inner: Arc<ClientInner>,
}

#[derive(Debug, Clone, Serialize)]
struct RunsCreateRequest {
    spec: WorkflowIntentSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<RunsCreateOptionsV0>,
}

#[derive(Debug, Clone, Serialize)]
struct RunsCreateFromPlanRequest {
    plan_hash: PlanHash,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<RunsCreateOptionsV0>,
}

#[derive(Debug, Clone, Serialize)]
struct RunsCreateOptionsV0 {
    idempotency_key: String,
}

#[derive(Debug, Clone, Default)]
pub struct RunsCreateOptions {
    pub session_id: Option<Uuid>,
    pub input: Option<HashMap<String, Value>>,
    pub stream: Option<bool>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunsCreateResponse {
    pub run_id: RunId,
    pub status: RunStatusV0,
    pub plan_hash: PlanHash,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunsGetResponse {
    pub run_id: RunId,
    pub status: RunStatusV0,
    pub plan_hash: PlanHash,
    pub cost_summary: RunCostSummaryV0,
    #[serde(default)]
    pub nodes: Vec<NodeResultV0>,
    #[serde(default)]
    pub outputs: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunsToolResultsRequest {
    pub node_id: NodeId,
    pub step: u64,
    pub request_id: RequestId,
    pub results: Vec<RunsToolResultItemV0>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunsToolCallV0 {
    pub id: ToolCallId,
    pub name: ToolName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunsToolResultItemV0 {
    pub tool_call: RunsToolCallV0,
    pub output: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunsToolResultsResponse {
    pub accepted: u64,
    pub status: RunStatusV0,
}

// RunsPendingToolsResponse, RunsPendingToolsNodeV0, RunsPendingToolCallV0 are
// generated from OpenAPI spec - imported from crate::generated above.

impl RunsClient {
    pub async fn create(&self, spec: WorkflowIntentSpec) -> Result<RunsCreateResponse> {
        self.create_with_options(spec, RunsCreateOptions::default())
            .await
    }

    pub async fn create_with_options(
        &self,
        spec: WorkflowIntentSpec,
        options: RunsCreateOptions,
    ) -> Result<RunsCreateResponse> {
        self.inner.ensure_auth()?;
        if options.session_id.is_some_and(|id| id.is_nil()) {
            return Err(Error::Validation(
                ValidationError::new("session_id is required").with_field("session_id"),
            ));
        }

        let options_payload = options.idempotency_key.as_ref().and_then(|key| {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(RunsCreateOptionsV0 {
                    idempotency_key: trimmed.to_string(),
                })
            }
        });

        let mut builder = self.inner.request(Method::POST, "/runs")?;
        builder = builder.json(&RunsCreateRequest {
            spec,
            input: options.input,
            session_id: options.session_id,
            stream: options.stream,
            options: options_payload,
        });
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, "/runs", None, None);
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }

    pub async fn create_with_session(
        &self,
        spec: WorkflowIntentSpec,
        session_id: Uuid,
    ) -> Result<RunsCreateResponse> {
        self.create_with_options(
            spec,
            RunsCreateOptions {
                session_id: Some(session_id),
                ..RunsCreateOptions::default()
            },
        )
        .await
    }

    /// Creates a workflow run using a precompiled plan hash.
    ///
    /// Use [`crate::WorkflowsClient::compile`] to compile a workflow spec and obtain a `plan_hash`,
    /// then use this method to start runs without re-compiling each time.
    /// This is useful for workflows that are run repeatedly with the same structure
    /// but different inputs.
    ///
    /// The plan_hash must have been compiled in the current server session;
    /// if the server has restarted since compilation, the plan will not be found
    /// and you'll need to recompile.
    pub async fn create_from_plan(&self, plan_hash: PlanHash) -> Result<RunsCreateResponse> {
        self.create_from_plan_with_options(plan_hash, RunsCreateOptions::default())
            .await
    }

    /// Creates a workflow run using a precompiled plan hash with options.
    ///
    /// See [`Self::create_from_plan`] for details on plan_hash usage.
    pub async fn create_from_plan_with_options(
        &self,
        plan_hash: PlanHash,
        options: RunsCreateOptions,
    ) -> Result<RunsCreateResponse> {
        self.inner.ensure_auth()?;
        // PlanHash derefs to String, use deref to check emptiness
        if (&*plan_hash).is_empty() {
            return Err(Error::Validation(
                ValidationError::new("plan_hash is required").with_field("plan_hash"),
            ));
        }
        if options.session_id.is_some_and(|id| id.is_nil()) {
            return Err(Error::Validation(
                ValidationError::new("session_id is required").with_field("session_id"),
            ));
        }

        let options_payload = options.idempotency_key.as_ref().and_then(|key| {
            let trimmed = key.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(RunsCreateOptionsV0 {
                    idempotency_key: trimmed.to_string(),
                })
            }
        });

        let mut builder = self.inner.request(Method::POST, "/runs")?;
        builder = builder.json(&RunsCreateFromPlanRequest {
            plan_hash,
            input: options.input,
            session_id: options.session_id,
            stream: options.stream,
            options: options_payload,
        });
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, "/runs", None, None);
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }

    pub async fn get(&self, run_id: RunId) -> Result<RunsGetResponse> {
        self.inner.ensure_auth()?;
        if run_id.0.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("run_id is required").with_field("run_id"),
            ));
        }
        let path = format!("/runs/{}", run_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        self.inner
            .execute_json(builder, Method::GET, None, ctx)
            .await
    }

    pub async fn pending_tools(&self, run_id: RunId) -> Result<RunsPendingToolsResponse> {
        self.inner.ensure_auth()?;
        if run_id.0.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("run_id is required").with_field("run_id"),
            ));
        }
        let path = format!("/runs/{}/pending-tools", run_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        self.inner
            .execute_json(builder, Method::GET, None, ctx)
            .await
    }

    pub async fn submit_tool_results(
        &self,
        run_id: RunId,
        req: RunsToolResultsRequest,
    ) -> Result<RunsToolResultsResponse> {
        self.inner.ensure_auth()?;
        if run_id.0.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("run_id is required").with_field("run_id"),
            ));
        }
        let path = format!("/runs/{}/tool-results", run_id);
        let mut builder = self.inner.request(Method::POST, &path)?;
        builder = builder.json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, &path, None, None);
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }

    #[cfg(feature = "streaming")]
    pub async fn stream_events(
        &self,
        run_id: RunId,
        after_seq: Option<i64>,
        limit: Option<i64>,
    ) -> Result<RunEventStreamHandle> {
        self.inner.ensure_auth()?;
        if run_id.0.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("run_id is required").with_field("run_id"),
            ));
        }
        let mut path = format!("/runs/{}/events", run_id);
        let mut q = vec![];
        if let Some(seq) = after_seq {
            if seq > 0 {
                q.push(("after_seq", seq.to_string()));
            }
        }
        if let Some(lim) = limit {
            if lim > 0 {
                q.push(("limit", lim.to_string()));
            }
        }
        if !q.is_empty() {
            path.push('?');
            path.push_str(
                &q.into_iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }

        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/x-ndjson"),
        )?;

        let retry = self.inner.retry.clone();
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp = self
            .inner
            .send_with_retry(builder, Method::GET, retry, ctx.clone())
            .await?;

        validate_ndjson_content_type(resp.headers(), resp.status().as_u16())?;

        let request_id = request_id_from_headers(resp.headers());
        Ok(RunEventStreamHandle::new(resp, request_id))
    }
}

#[cfg(feature = "streaming")]
pub struct RunEventStreamHandle {
    request_id: Option<String>,
    stream: Pin<Box<dyn Stream<Item = Result<RunEventV0>> + Send>>,
    cancelled: Arc<AtomicBool>,
}

#[cfg(feature = "streaming")]
impl RunEventStreamHandle {
    fn new(response: reqwest::Response, request_id: Option<String>) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream = build_run_events_stream(response, cancelled.clone());
        Self {
            request_id,
            stream: Box::pin(stream),
            cancelled,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

#[cfg(feature = "streaming")]
impl Drop for RunEventStreamHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(feature = "streaming")]
impl Stream for RunEventStreamHandle {
    type Item = Result<RunEventV0>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = unsafe { self.map_unchecked_mut(|s| &mut s.stream) };
        stream.poll_next(cx)
    }
}

/// State for run event stream processing.
///
/// Simpler than NdjsonStreamState since runs don't need timeouts or telemetry.
#[cfg(feature = "streaming")]
struct RunEventStreamState<B> {
    body: B,
    buffer: String,
    cancelled: Arc<AtomicBool>,
    pending: VecDeque<RunEventV0>,
}

#[cfg(feature = "streaming")]
impl<B> RunEventStreamState<B> {
    fn new(body: B, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            body,
            buffer: String::new(),
            cancelled,
            pending: VecDeque::new(),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(feature = "streaming")]
fn build_run_events_stream(
    response: reqwest::Response,
    cancelled: Arc<AtomicBool>,
) -> impl Stream<Item = Result<RunEventV0>> + Send {
    let body = response.bytes_stream();
    let state = RunEventStreamState::new(body, cancelled);

    stream::unfold(state, |mut state| async move {
        loop {
            // Check cancellation
            if state.is_cancelled() {
                return None;
            }

            // Return pending events
            if let Some(ev) = state.pending.pop_front() {
                return Some((Ok(ev), state));
            }

            match state.body.next().await {
                Some(Ok(chunk)) => {
                    // Parse UTF-8
                    let text = match String::from_utf8(chunk.to_vec()) {
                        Ok(s) => s,
                        Err(e) => {
                            let err = Error::StreamProtocol {
                                message: format!("invalid UTF-8 in stream: {}", e),
                                raw_data: None,
                            };
                            return Some((Err(err), state));
                        }
                    };

                    // Parse NDJSON lines
                    state.buffer.push_str(&text);
                    let (events, remainder) = consume_ndjson_buffer(&state.buffer);
                    state.buffer = remainder;

                    // Process events
                    for raw in events {
                        match parse_run_event(&raw.data) {
                            Ok(ev) => state.pending.push_back(ev),
                            Err(err) => return Some((Err(err), state)),
                        }
                    }
                    continue;
                }
                Some(Err(err)) => {
                    let error = Error::Transport(TransportError {
                        kind: classify_reqwest_error(&err),
                        message: err.to_string(),
                        source: Some(err),
                        retries: None,
                    });
                    return Some((Err(error), state));
                }
                None => {
                    // Stream ended, process remaining buffer
                    let (events, _) = consume_ndjson_buffer(&state.buffer);
                    state.buffer.clear();

                    for raw in events {
                        match parse_run_event(&raw.data) {
                            Ok(ev) => state.pending.push_back(ev),
                            Err(err) => return Some((Err(err), state)),
                        }
                    }

                    if let Some(ev) = state.pending.pop_front() {
                        return Some((Ok(ev), state));
                    }
                    return None;
                }
            }
        }
    })
}

/// Parses and validates a run event from raw JSON data.
/// Extracted for SRP: separates parsing/validation from stream management.
#[cfg(feature = "streaming")]
fn parse_run_event(raw_data: &str) -> Result<RunEventV0> {
    let ev: RunEventV0 = serde_json::from_str(raw_data).map_err(|err| Error::StreamProtocol {
        message: format!("failed to parse run event: {err}"),
        raw_data: Some(raw_data.to_string()),
    })?;
    ev.validate().map_err(|err| Error::StreamProtocol {
        message: format!("invalid run event: {err}"),
        raw_data: Some(raw_data.to_string()),
    })?;
    Ok(ev)
}
