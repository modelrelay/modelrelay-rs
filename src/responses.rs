use std::time::Duration;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::client::ResponsesClient;
use crate::errors::{APIError, Error, Result, TransportError, TransportErrorKind, ValidationError};
use crate::http::{ResponseOptions, StreamTimeouts};
#[cfg(feature = "streaming")]
use crate::ndjson::StreamHandle;
use crate::types::{
    InputItem, MessageRole, Model, OutputFormat, Response, ResponseRequest, Tool, ToolChoice,
};
use crate::workflow::ProviderId;
use crate::RetryConfig;

#[cfg(feature = "blocking")]
use crate::blocking::BlockingResponsesClient;
#[cfg(all(feature = "blocking", feature = "streaming"))]
use crate::blocking::BlockingStreamHandle;

/// HTTP header name for customer-attributed requests.
pub const CUSTOMER_ID_HEADER: &str = "X-ModelRelay-Customer-Id";

#[cfg(feature = "streaming")]
fn validate_structured_output_format(format: Option<&OutputFormat>) -> Result<()> {
    match format {
        Some(f) if f.is_structured() => Ok(()),
        Some(_) => Err(Error::Validation(
            ValidationError::new("output_format must be structured (type=json_schema)")
                .with_field("output_format.type"),
        )),
        None => Err(Error::Validation(
            ValidationError::new("output_format is required for structured streaming")
                .with_field("output_format"),
        )),
    }
}

trait OptionsBuilder {
    fn request_id(&self) -> Option<&str>;
    fn headers(&self) -> &[(String, String)];
    fn timeout(&self) -> Option<Duration>;
    fn stream_timeouts(&self) -> StreamTimeouts;
    fn retry(&self) -> Option<&RetryConfig>;

    fn build_options(&self) -> ResponseOptions {
        let mut opts = ResponseOptions::default();
        if let Some(req_id) = self.request_id() {
            opts = opts.with_request_id(req_id.to_string());
        }
        for (k, v) in self.headers() {
            opts = opts.with_header(k.clone(), v.clone());
        }
        if let Some(timeout) = self.timeout() {
            opts = opts.with_timeout(timeout);
        }
        opts = opts.with_stream_timeouts(self.stream_timeouts());
        if let Some(retry) = self.retry() {
            opts = opts.with_retry(retry.clone());
        }
        opts
    }
}

/// Request payload for POST /responses (pure data, no transport options).
///
/// This struct holds only the fields that go in the HTTP request body,
/// separating them from transport-level concerns like timeouts and headers.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResponsePayload {
    pub provider: Option<ProviderId>,
    pub model: Option<Model>,
    pub input: Vec<InputItem>,
    pub output_format: Option<OutputFormat>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub stop: Option<Vec<String>>,
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,
}

impl ResponsePayload {
    /// Convert to the internal request type.
    pub fn into_request(self) -> ResponseRequest {
        ResponseRequest {
            provider: self.provider,
            model: self.model,
            input: self.input,
            output_format: self.output_format,
            max_output_tokens: self.max_output_tokens,
            temperature: self.temperature,
            stop: self.stop,
            tools: self.tools,
            tool_choice: self.tool_choice,
        }
    }
}

/// Builder for `POST /responses` (async).
///
/// Separates concerns:
/// - `payload`: Request body data (model, input, tools, etc.)
/// - Transport options: HTTP-level config (headers, timeouts, retry)
#[derive(Clone, Debug, Default)]
pub struct ResponseBuilder {
    /// Request payload (what goes in the HTTP body).
    pub(crate) payload: ResponsePayload,
    /// Transport options below.
    pub(crate) request_id: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) stream_timeouts: StreamTimeouts,
    pub(crate) retry: Option<RetryConfig>,
}

impl OptionsBuilder for ResponseBuilder {
    fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
    fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
    fn timeout(&self) -> Option<Duration> {
        self.timeout
    }
    fn stream_timeouts(&self) -> StreamTimeouts {
        self.stream_timeouts
    }
    fn retry(&self) -> Option<&RetryConfig> {
        self.retry.as_ref()
    }
}

impl ResponseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a "chat-like" text prompt builder (system + user).
    ///
    /// This is a thin convenience wrapper over `ResponseBuilder` for the common
    /// text-only path. You must still set either:
    /// - `.model(...)`, or
    /// - `.customer_id(...)` (to let the backend select a model)
    ///
    /// To execute and get assistant text, use `.send_text(...)`.
    #[must_use]
    pub fn text_prompt(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self::new().system(system).user(user)
    }

    // =========================================================================
    // Payload setters (request body data)
    // =========================================================================

    /// Set provider (optional).
    #[must_use]
    pub fn provider(mut self, provider: ProviderId) -> Self {
        self.payload.provider = Some(provider);
        self
    }

    /// Set model (required unless `customer_id(...)` is provided).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model>) -> Self {
        self.payload.model = Some(model.into());
        self
    }

    /// Replace the entire input list.
    #[must_use]
    pub fn input(mut self, input: Vec<InputItem>) -> Self {
        self.payload.input = input;
        self
    }

    /// Append a single input item.
    #[must_use]
    pub fn item(mut self, item: InputItem) -> Self {
        self.payload.input.push(item);
        self
    }

    /// Append a message input item (text content).
    #[must_use]
    pub fn message(mut self, role: MessageRole, content: impl Into<String>) -> Self {
        self.payload.input.push(InputItem::message(role, content));
        self
    }

    #[must_use]
    pub fn system(self, content: impl Into<String>) -> Self {
        self.message(MessageRole::System, content)
    }

    #[must_use]
    pub fn user(self, content: impl Into<String>) -> Self {
        self.message(MessageRole::User, content)
    }

    #[must_use]
    pub fn assistant(self, content: impl Into<String>) -> Self {
        self.message(MessageRole::Assistant, content)
    }

    /// Append a tool result message for a given tool call id.
    #[must_use]
    pub fn tool_result(self, tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        self.item(InputItem::tool_result(tool_call_id, content))
    }

    #[must_use]
    pub fn output_format(mut self, output_format: OutputFormat) -> Self {
        self.payload.output_format = Some(output_format);
        self
    }

    #[must_use]
    pub fn max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.payload.max_output_tokens = Some(max_output_tokens);
        self
    }

    #[must_use]
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.payload.temperature = Some(temperature);
        self
    }

    #[must_use]
    pub fn stop(mut self, stop: Vec<String>) -> Self {
        self.payload.stop = Some(stop);
        self
    }

    #[must_use]
    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.payload.tools = Some(tools);
        self
    }

    #[must_use]
    pub fn tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.payload.tool_choice = Some(tool_choice);
        self
    }

    // =========================================================================
    // Transport options setters (HTTP-level config)
    // =========================================================================

    /// Set customer id header (model can be omitted).
    #[must_use]
    pub fn customer_id(mut self, customer_id: impl Into<String>) -> Self {
        self.headers
            .push((CUSTOMER_ID_HEADER.to_string(), customer_id.into()));
        self
    }

    #[must_use]
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    #[must_use]
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set stream TTFT timeout (time-to-first-content).
    #[must_use]
    pub fn stream_ttft_timeout(mut self, timeout: Duration) -> Self {
        self.stream_timeouts.ttft = Some(timeout);
        self
    }

    /// Set stream idle timeout (max time without receiving bytes).
    #[must_use]
    pub fn stream_idle_timeout(mut self, timeout: Duration) -> Self {
        self.stream_timeouts.idle = Some(timeout);
        self
    }

    /// Set stream total timeout (overall stream deadline).
    #[must_use]
    pub fn stream_total_timeout(mut self, timeout: Duration) -> Self {
        self.stream_timeouts.total = Some(timeout);
        self
    }

    #[must_use]
    pub fn retry(mut self, retry: RetryConfig) -> Self {
        self.retry = Some(retry);
        self
    }

    // =========================================================================
    // Build helpers
    // =========================================================================

    pub(crate) fn build_request(&self) -> Result<ResponseRequest> {
        Ok(self.payload.clone().into_request())
    }

    pub async fn send(self, client: &ResponsesClient) -> Result<Response> {
        let req = self.build_request()?;
        let options = self.build_options();
        client.create(req, options).await
    }

    /// Send the request and return concatenated assistant text.
    ///
    /// Returns an `EmptyResponse` transport error if the response contains no
    /// assistant text output.
    pub async fn send_text(self, client: &ResponsesClient) -> Result<String> {
        let response = self.send(client).await?;
        assistant_text_required(&response)
    }

    #[cfg(feature = "streaming")]
    pub async fn stream(self, client: &ResponsesClient) -> Result<StreamHandle> {
        let req = self.build_request()?;
        let options = self.build_options();
        client.stream(req, options).await
    }

    /// Convenience helper to stream text deltas directly (async).
    #[cfg(feature = "streaming")]
    pub async fn stream_deltas(
        self,
        client: &ResponsesClient,
    ) -> Result<std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<String>> + Send>>> {
        let stream = self.stream(client).await?;
        Ok(Box::pin(
            ResponseStreamAdapter::<StreamHandle>::new(stream).into_stream(),
        ))
    }

    /// Stream structured JSON over NDJSON (async).
    #[cfg(feature = "streaming")]
    pub async fn stream_json<T>(self, client: &ResponsesClient) -> Result<StructuredJSONStream<T>>
    where
        T: DeserializeOwned,
    {
        validate_structured_output_format(self.payload.output_format.as_ref())?;
        let stream = self.stream(client).await?;
        Ok(StructuredJSONStream::new(stream))
    }

    /// Stream structured JSON over NDJSON (blocking).
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_json_blocking<T>(
        self,
        client: &BlockingResponsesClient,
    ) -> Result<BlockingStructuredJSONStream<T>>
    where
        T: DeserializeOwned,
    {
        validate_structured_output_format(self.payload.output_format.as_ref())?;
        let stream = self.stream_blocking(client)?;
        Ok(BlockingStructuredJSONStream::new(stream))
    }

    #[cfg(feature = "blocking")]
    pub fn send_blocking(self, client: &BlockingResponsesClient) -> Result<Response> {
        let req = self.build_request()?;
        let options = self.build_options();
        client.create(req, options)
    }

    /// Send the request and return concatenated assistant text (blocking).
    ///
    /// Returns an `EmptyResponse` transport error if the response contains no
    /// assistant text output.
    #[cfg(feature = "blocking")]
    pub fn send_text_blocking(self, client: &BlockingResponsesClient) -> Result<String> {
        let response = self.send_blocking(client)?;
        assistant_text_required(&response)
    }

    #[cfg(feature = "blocking")]
    pub fn stream_blocking(self, client: &BlockingResponsesClient) -> Result<BlockingStreamHandle> {
        let req = self.build_request()?;
        let options = self.build_options();
        client.stream(req, options)
    }

    /// Convenience helper to stream text deltas directly (blocking).
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_text_deltas_blocking(
        self,
        client: &BlockingResponsesClient,
    ) -> Result<impl Iterator<Item = Result<String>>> {
        let stream = self.stream_blocking(client)?;
        Ok(ResponseStreamAdapter::<BlockingStreamHandle>::new(stream).into_iter())
    }

    /// Convenience wrapper around `schemars` to build `output_format` from a type.
    #[must_use]
    pub fn structured<T>(self) -> crate::structured::StructuredResponseBuilder<T>
    where
        T: JsonSchema + DeserializeOwned,
    {
        crate::structured::StructuredResponseBuilder::new(self)
    }
}

/// Adapter that yields only text deltas from a stream handle.
pub struct ResponseStreamAdapter<S> {
    inner: S,
}

/// Computes the incremental text delta from accumulated text.
///
/// This pure function handles the deduplication logic for streaming text:
/// - If `next` extends `accumulated`, returns only the new portion
/// - If `next` is a prefix of `accumulated`, returns empty (no new content)
/// - Otherwise, returns `next` and appends to accumulated
///
/// Returns `(delta_to_emit, new_accumulated)`.
fn compute_text_delta(accumulated: String, next: String) -> (String, String) {
    if next.starts_with(&accumulated) {
        // next extends accumulated - emit only the new part
        let delta = next[accumulated.len()..].to_string();
        (delta, next)
    } else if accumulated.starts_with(&next) {
        // next is a prefix of accumulated - no new content
        (String::new(), next)
    } else {
        // Unrelated - append next to accumulated
        let mut new_acc = accumulated;
        new_acc.push_str(&next);
        (next, new_acc)
    }
}

fn assistant_text_required(response: &Response) -> Result<String> {
    let text = response.text();
    if text.trim().is_empty() {
        return Err(Error::Transport(TransportError {
            kind: TransportErrorKind::EmptyResponse,
            message: "response contained no assistant text output".to_string(),
            source: None,
            retries: None,
        }));
    }
    Ok(text)
}

#[cfg(feature = "streaming")]
impl ResponseStreamAdapter<StreamHandle> {
    pub fn new(inner: StreamHandle) -> Self {
        Self { inner }
    }

    pub fn into_stream(self) -> impl futures_core::Stream<Item = Result<String>> + Send + 'static {
        use futures_util::StreamExt;
        futures_util::stream::unfold(
            (self.inner, String::new()),
            |(mut inner, mut accumulated)| async move {
                while let Some(item) = inner.next().await {
                    match item {
                        Ok(evt) => {
                            let is_text_evt = evt.kind
                                == crate::types::StreamEventKind::MessageDelta
                                || evt.kind == crate::types::StreamEventKind::MessageStop;
                            if is_text_evt {
                                if let Some(next) = evt.text_delta {
                                    let (delta, new_accumulated) =
                                        compute_text_delta(accumulated, next);
                                    accumulated = new_accumulated;
                                    if !delta.is_empty() {
                                        return Some((Ok(delta), (inner, accumulated)));
                                    }
                                }
                            }
                        }
                        Err(e) => return Some((Err(e), (inner, accumulated))),
                    }
                }
                None
            },
        )
    }
}

#[cfg(all(feature = "blocking", feature = "streaming"))]
impl ResponseStreamAdapter<BlockingStreamHandle> {
    pub fn new(inner: BlockingStreamHandle) -> Self {
        Self { inner }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn into_iter(mut self) -> impl Iterator<Item = Result<String>> {
        let mut accumulated = String::new();
        std::iter::from_fn(move || loop {
            match self.inner.next() {
                Ok(Some(evt)) => {
                    let is_text_evt = evt.kind == crate::types::StreamEventKind::MessageDelta
                        || evt.kind == crate::types::StreamEventKind::MessageStop;
                    if is_text_evt {
                        if let Some(next) = evt.text_delta {
                            let (delta, new_accumulated) =
                                compute_text_delta(std::mem::take(&mut accumulated), next);
                            accumulated = new_accumulated;
                            if !delta.is_empty() {
                                return Some(Ok(delta));
                            }
                        }
                    }
                    continue;
                }
                Ok(None) => return None,
                Err(e) => return Some(Err(e)),
            }
        })
    }
}

/// Kind of structured record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredRecordKind {
    Update,
    Completion,
}

/// Structured JSON event (parsed from NDJSON envelope).
#[derive(Debug, Clone)]
pub struct StructuredJSONEvent<T> {
    pub kind: StructuredRecordKind,
    pub payload: T,
    pub request_id: Option<String>,
    pub complete_fields: Vec<String>,
}

enum ParsedStructuredRecord<T> {
    Event(StructuredJSONEvent<T>),
    Error(APIError),
    Skip,
}

fn parse_structured_record<T>(
    evt: &crate::types::StreamEvent,
    fallback_request_id: Option<&str>,
) -> Result<ParsedStructuredRecord<T>>
where
    T: DeserializeOwned,
{
    let Some(value) = evt.data.as_ref().and_then(|v| v.as_object()) else {
        return Ok(ParsedStructuredRecord::Skip);
    };
    let record_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match record_type {
        "update" | "completion" => {
            let kind = if record_type == "completion" {
                StructuredRecordKind::Completion
            } else {
                StructuredRecordKind::Update
            };
            let payload = value
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let payload: T = serde_json::from_value(payload).map_err(Error::Serialization)?;
            let request_id = evt
                .request_id
                .clone()
                .or_else(|| fallback_request_id.map(|s| s.to_string()));
            let complete_fields = value
                .get("complete_fields")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            Ok(ParsedStructuredRecord::Event(StructuredJSONEvent {
                kind,
                payload,
                request_id,
                complete_fields,
            }))
        }
        "error" => {
            let code = value
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let message = value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("structured stream error")
                .to_string();
            let status = value
                .get("status")
                .and_then(|v| v.as_u64())
                .map(|v| v as u16)
                .unwrap_or(500);
            let request_id = evt
                .request_id
                .clone()
                .or_else(|| fallback_request_id.map(|s| s.to_string()));
            Ok(ParsedStructuredRecord::Error(APIError {
                status,
                code,
                message,
                request_id,
                fields: Vec::new(),
                retries: None,
                raw_body: None,
            }))
        }
        _ => Ok(ParsedStructuredRecord::Skip),
    }
}

/// Helper over NDJSON streaming events to yield structured JSON payloads.
#[cfg(feature = "streaming")]
pub struct StructuredJSONStream<T> {
    inner: StreamHandle,
    finished: bool,
    saw_completion: bool,
    _marker: std::marker::PhantomData<T>,
}

#[cfg(feature = "streaming")]
impl<T> StructuredJSONStream<T>
where
    T: DeserializeOwned,
{
    pub fn new(stream: StreamHandle) -> Self {
        Self {
            inner: stream,
            finished: false,
            saw_completion: false,
            _marker: std::marker::PhantomData,
        }
    }

    pub async fn next(&mut self) -> Result<Option<StructuredJSONEvent<T>>> {
        use futures_util::StreamExt;

        if self.finished {
            return Ok(None);
        }

        while let Some(item) = self.inner.next().await {
            let evt = item?;
            match parse_structured_record::<T>(&evt, self.inner.request_id())? {
                ParsedStructuredRecord::Event(event) => {
                    if matches!(event.kind, StructuredRecordKind::Completion) {
                        self.saw_completion = true;
                    }
                    return Ok(Some(event));
                }
                ParsedStructuredRecord::Error(api_error) => {
                    self.saw_completion = true;
                    return Err(api_error.into());
                }
                ParsedStructuredRecord::Skip => continue,
            }
        }

        self.finished = true;
        if !self.saw_completion {
            return Err(Error::Transport(TransportError {
                kind: TransportErrorKind::Request,
                message: "structured stream ended without completion or error".to_string(),
                source: None,
                retries: None,
            }));
        }
        Ok(None)
    }

    pub async fn collect(mut self) -> Result<T> {
        let mut last: Option<T> = None;
        while let Some(event) = self.next().await? {
            if matches!(event.kind, StructuredRecordKind::Completion) {
                return Ok(event.payload);
            }
            last = Some(event.payload);
        }
        match last {
            Some(payload) => Ok(payload),
            None => Err(Error::Transport(TransportError {
                kind: TransportErrorKind::Request,
                message: "structured stream ended without completion or error".to_string(),
                source: None,
                retries: None,
            })),
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.inner.request_id()
    }
}

/// Blocking helper over NDJSON streaming events to yield structured JSON payloads.
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub struct BlockingStructuredJSONStream<T> {
    inner: BlockingStreamHandle,
    finished: bool,
    saw_completion: bool,
    _marker: std::marker::PhantomData<T>,
}

#[cfg(all(feature = "blocking", feature = "streaming"))]
impl<T> BlockingStructuredJSONStream<T>
where
    T: DeserializeOwned,
{
    pub fn new(stream: BlockingStreamHandle) -> Self {
        Self {
            inner: stream,
            finished: false,
            saw_completion: false,
            _marker: std::marker::PhantomData,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<StructuredJSONEvent<T>>> {
        if self.finished {
            return Ok(None);
        }

        while let Some(evt) = self.inner.next()? {
            match parse_structured_record::<T>(&evt, self.inner.request_id())? {
                ParsedStructuredRecord::Event(event) => {
                    if matches!(event.kind, StructuredRecordKind::Completion) {
                        self.saw_completion = true;
                    }
                    return Ok(Some(event));
                }
                ParsedStructuredRecord::Error(api_error) => {
                    self.saw_completion = true;
                    return Err(api_error.into());
                }
                ParsedStructuredRecord::Skip => continue,
            }
        }

        self.finished = true;
        if !self.saw_completion {
            return Err(Error::Transport(TransportError {
                kind: TransportErrorKind::Request,
                message: "structured stream ended without completion or error".to_string(),
                source: None,
                retries: None,
            }));
        }
        Ok(None)
    }

    pub fn collect(mut self) -> Result<T> {
        let mut last: Option<T> = None;
        while let Some(event) = self.next()? {
            if matches!(event.kind, StructuredRecordKind::Completion) {
                return Ok(event.payload);
            }
            last = Some(event.payload);
        }
        match last {
            Some(payload) => Ok(payload),
            None => Err(Error::Transport(TransportError {
                kind: TransportErrorKind::Request,
                message: "structured stream ended without completion or error".to_string(),
                source: None,
                retries: None,
            })),
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.inner.request_id()
    }
}
