use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use reqwest::{header::CONTENT_TYPE, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    client::ClientInner,
    core::consume_ndjson_buffer,
    errors::{Error, Result, TransportError, TransportErrorKind, ValidationError},
    http::{request_id_from_headers, HeaderList},
    workflow::{NodeResultV0, PlanHash, RunEventV0, RunId, RunStatusV0, WorkflowSpecV0},
};

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
    spec: WorkflowSpecV0,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunsCreateResponse {
    pub run_id: RunId,
    pub status: RunStatusV0,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunsGetResponse {
    pub run_id: RunId,
    pub status: RunStatusV0,
    pub plan_hash: PlanHash,
    #[serde(default)]
    pub nodes: Vec<NodeResultV0>,
    #[serde(default)]
    pub outputs: HashMap<String, Value>,
}

impl RunsClient {
    pub async fn schema_v0(&self) -> Result<Value> {
        self.inner.ensure_auth()?;
        let path = "/schemas/workflow_v0.schema.json";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/schema+json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        self.inner
            .execute_json(builder, Method::GET, None, ctx)
            .await
    }

    pub async fn create(&self, spec: WorkflowSpecV0) -> Result<RunsCreateResponse> {
        self.inner.ensure_auth()?;
        let mut builder = self.inner.request(Method::POST, "/runs")?;
        builder = builder.json(&RunsCreateRequest { spec });
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

        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_lowercase());
        let is_ndjson = content_type
            .as_deref()
            .map(|ct| {
                ct.starts_with("application/x-ndjson") || ct.starts_with("application/ndjson")
            })
            .unwrap_or(false);
        if !is_ndjson {
            return Err(Error::StreamContentType {
                expected: "application/x-ndjson",
                received: content_type.unwrap_or_else(|| "<missing>".to_string()),
                status: resp.status().as_u16(),
            });
        }

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

#[cfg(feature = "streaming")]
fn build_run_events_stream(
    response: reqwest::Response,
    cancelled: Arc<AtomicBool>,
) -> impl Stream<Item = Result<RunEventV0>> + Send {
    let body = response.bytes_stream();
    let state = (
        body,
        String::new(),
        cancelled,
        VecDeque::<RunEventV0>::new(),
    );

    stream::unfold(state, |state| async move {
        let (mut body, mut buffer, cancelled, mut pending) = state;

        loop {
            if cancelled.load(Ordering::SeqCst) {
                return None;
            }

            if let Some(ev) = pending.pop_front() {
                return Some((Ok(ev), (body, buffer, cancelled, pending)));
            }

            match body.next().await {
                Some(Ok(chunk)) => {
                    buffer.push_str(&String::from_utf8_lossy(&chunk));
                    let (events, remainder) = consume_ndjson_buffer(&buffer);
                    buffer = remainder;
                    for raw in events {
                        let parsed = serde_json::from_str::<RunEventV0>(&raw.data)
                            .map_err(|err| Error::StreamProtocol {
                                message: format!("failed to parse run event: {err}"),
                                raw_data: Some(raw.data.clone()),
                            })
                            .and_then(|ev| {
                                ev.validate().map_err(|err| Error::StreamProtocol {
                                    message: format!("invalid run event: {err}"),
                                    raw_data: Some(raw.data.clone()),
                                })?;
                                Ok(ev)
                            });
                        match parsed {
                            Ok(ev) => pending.push_back(ev),
                            Err(err) => {
                                return Some((Err(err), (body, buffer, cancelled, pending)));
                            }
                        }
                    }
                    continue;
                }
                Some(Err(err)) => {
                    let error = Error::Transport(TransportError {
                        kind: if err.is_timeout() {
                            TransportErrorKind::Timeout
                        } else if err.is_connect() {
                            TransportErrorKind::Connect
                        } else if err.is_request() {
                            TransportErrorKind::Request
                        } else {
                            TransportErrorKind::Other
                        },
                        message: err.to_string(),
                        source: Some(err),
                        retries: None,
                    });
                    return Some((Err(error), (body, buffer, cancelled, pending)));
                }
                None => {
                    let (events, _) = consume_ndjson_buffer(&buffer);
                    buffer.clear();
                    for raw in events {
                        match serde_json::from_str::<RunEventV0>(&raw.data)
                            .map_err(|err| Error::StreamProtocol {
                                message: format!("failed to parse run event: {err}"),
                                raw_data: Some(raw.data.clone()),
                            })
                            .and_then(|ev| {
                                ev.validate().map_err(|err| Error::StreamProtocol {
                                    message: format!("invalid run event: {err}"),
                                    raw_data: Some(raw.data.clone()),
                                })?;
                                Ok(ev)
                            }) {
                            Ok(ev) => pending.push_back(ev),
                            Err(err) => {
                                return Some((Err(err), (body, buffer, cancelled, pending)));
                            }
                        }
                    }
                    if let Some(ev) = pending.pop_front() {
                        return Some((Ok(ev), (body, buffer, cancelled, pending)));
                    }
                    return None;
                }
            }
        }
    })
}
