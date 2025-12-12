use std::time::Duration;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::client::ResponsesClient;
use crate::errors::{APIError, Error, Result, TransportError, TransportErrorKind, ValidationError};
use crate::http::ResponseOptions;
#[cfg(feature = "streaming")]
use crate::ndjson::StreamHandle;
use crate::types::{
    InputItem, MessageRole, Model, OutputFormat, Response, ResponseRequest, Tool, ToolChoice,
};
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
        if let Some(retry) = self.retry() {
            opts = opts.with_retry(retry.clone());
        }
        opts
    }
}

/// Builder for `POST /responses` (async).
#[derive(Clone, Debug, Default)]
pub struct ResponseBuilder {
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<Model>,
    pub(crate) input: Vec<InputItem>,
    pub(crate) output_format: Option<OutputFormat>,
    pub(crate) max_output_tokens: Option<i64>,
    pub(crate) temperature: Option<f64>,
    pub(crate) stop: Option<Vec<String>>,
    pub(crate) tools: Option<Vec<Tool>>,
    pub(crate) tool_choice: Option<ToolChoice>,
    pub(crate) request_id: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Option<Duration>,
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
    fn retry(&self) -> Option<&RetryConfig> {
        self.retry.as_ref()
    }
}

impl ResponseBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set provider (optional).
    #[must_use]
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Set model (required unless `customer_id(...)` is provided).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Replace the entire input list.
    #[must_use]
    pub fn input(mut self, input: Vec<InputItem>) -> Self {
        self.input = input;
        self
    }

    /// Append a single input item.
    #[must_use]
    pub fn item(mut self, item: InputItem) -> Self {
        self.input.push(item);
        self
    }

    /// Append a message input item (text content).
    #[must_use]
    pub fn message(mut self, role: MessageRole, content: impl Into<String>) -> Self {
        self.input.push(InputItem::message(role, content));
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
        self.output_format = Some(output_format);
        self
    }

    #[must_use]
    pub fn max_output_tokens(mut self, max_output_tokens: i64) -> Self {
        self.max_output_tokens = Some(max_output_tokens);
        self
    }

    #[must_use]
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    #[must_use]
    pub fn stop(mut self, stop: Vec<String>) -> Self {
        self.stop = Some(stop);
        self
    }

    #[must_use]
    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = Some(tools);
        self
    }

    #[must_use]
    pub fn tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

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

    #[must_use]
    pub fn retry(mut self, retry: RetryConfig) -> Self {
        self.retry = Some(retry);
        self
    }

    pub(crate) fn build_request(&self) -> Result<ResponseRequest> {
        Ok(ResponseRequest {
            provider: self.provider.clone(),
            model: self.model.clone(),
            input: self.input.clone(),
            output_format: self.output_format.clone(),
            max_output_tokens: self.max_output_tokens,
            temperature: self.temperature,
            stop: self.stop.clone(),
            tools: self.tools.clone(),
            tool_choice: self.tool_choice.clone(),
        })
    }

    pub async fn send(self, client: &ResponsesClient) -> Result<Response> {
        let req = self.build_request()?;
        let options = self.build_options();
        client.create(req, options).await
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
        validate_structured_output_format(self.output_format.as_ref())?;
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
        validate_structured_output_format(self.output_format.as_ref())?;
        let stream = self.stream_blocking(client)?;
        Ok(BlockingStructuredJSONStream::new(stream))
    }

    #[cfg(feature = "blocking")]
    pub fn send_blocking(self, client: &BlockingResponsesClient) -> Result<Response> {
        let req = self.build_request()?;
        let options = self.build_options();
        client.create(req, options)
    }

    #[cfg(feature = "blocking")]
    pub fn stream_blocking(self, client: &BlockingResponsesClient) -> Result<BlockingStreamHandle> {
        let req = self.build_request()?;
        let options = self.build_options();
        client.stream(req, options)
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

#[cfg(feature = "streaming")]
impl ResponseStreamAdapter<StreamHandle> {
    pub fn new(inner: StreamHandle) -> Self {
        Self { inner }
    }

    pub fn into_stream(self) -> impl futures_core::Stream<Item = Result<String>> + Send + 'static {
        use futures_util::StreamExt;
        futures_util::stream::unfold(self.inner, |mut inner| async move {
            while let Some(item) = inner.next().await {
                match item {
                    Ok(evt) => {
                        if evt.kind == crate::types::StreamEventKind::MessageDelta {
                            if let Some(delta) = evt.text_delta {
                                return Some((Ok(delta), inner));
                            }
                        }
                    }
                    Err(e) => return Some((Err(e), inner)),
                }
            }
            None
        })
    }
}

#[cfg(all(feature = "blocking", feature = "streaming"))]
impl ResponseStreamAdapter<BlockingStreamHandle> {
    pub fn new(inner: BlockingStreamHandle) -> Self {
        Self { inner }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn into_iter(mut self) -> impl Iterator<Item = Result<String>> {
        std::iter::from_fn(move || loop {
            match self.inner.next() {
                Ok(Some(evt)) => {
                    if evt.kind == crate::types::StreamEventKind::MessageDelta {
                        if let Some(delta) = evt.text_delta.clone() {
                            return Some(Ok(delta));
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
