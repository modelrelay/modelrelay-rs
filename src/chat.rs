#![cfg(any(feature = "client", feature = "blocking"))]

use std::collections::HashMap;
use std::time::Duration;

use crate::errors::{APIError, Error, Result, TransportError, TransportErrorKind, ValidationError};
#[cfg(feature = "streaming")]
use crate::types::StreamEventKind;
use crate::types::{
    Model, ProxyMessage, ProxyRequest, ProxyResponse, ResponseFormat, StopReason, Usage,
};

#[cfg(feature = "blocking")]
use crate::blocking::BlockingLLMClient;
#[cfg(all(feature = "blocking", feature = "streaming"))]
use crate::blocking::BlockingProxyHandle;
#[cfg(feature = "client")]
use crate::client::LLMClient;
#[cfg(all(feature = "client", feature = "streaming"))]
use crate::sse::StreamHandle;

#[cfg(any(feature = "client", feature = "blocking"))]
use crate::{ProxyOptions, RetryConfig};
#[cfg(all(feature = "client", feature = "streaming"))]
use futures_util::stream;
use schemars::JsonSchema;
#[cfg(all(feature = "client", feature = "streaming"))]
use serde::de::DeserializeOwned;

/// Macro to implement shared builder methods for chat request builders.
///
/// Both `ChatRequestBuilder` and `CustomerChatRequestBuilder` share identical
/// methods for setting messages, parameters, and options. This macro eliminates
/// the duplication.
macro_rules! impl_chat_builder_common {
    ($builder:ty) => {
        impl $builder {
            /// Add a message with the given role and content.
            pub fn message(
                mut self,
                role: crate::types::MessageRole,
                content: impl Into<String>,
            ) -> Self {
                self.messages.push(ProxyMessage {
                    role,
                    content: content.into(),
                    tool_calls: None,
                    tool_call_id: None,
                });
                self
            }

            /// Add a system message.
            pub fn system(self, content: impl Into<String>) -> Self {
                self.message(crate::types::MessageRole::System, content)
            }

            /// Add a user message.
            pub fn user(self, content: impl Into<String>) -> Self {
                self.message(crate::types::MessageRole::User, content)
            }

            /// Add an assistant message.
            pub fn assistant(self, content: impl Into<String>) -> Self {
                self.message(crate::types::MessageRole::Assistant, content)
            }

            /// Set the full message list, replacing any existing messages.
            pub fn messages(mut self, messages: Vec<ProxyMessage>) -> Self {
                self.messages = messages;
                self
            }

            /// Set the maximum number of tokens to generate.
            pub fn max_tokens(mut self, max_tokens: i64) -> Self {
                self.max_tokens = Some(max_tokens);
                self
            }

            /// Set the sampling temperature.
            pub fn temperature(mut self, temperature: f64) -> Self {
                self.temperature = Some(temperature);
                self
            }

            /// Set request metadata.
            pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
                self.metadata = Some(metadata);
                self
            }

            /// Add a single metadata entry. Empty keys or values are ignored.
            pub fn metadata_entry(
                mut self,
                key: impl Into<String>,
                value: impl Into<String>,
            ) -> Self {
                let key = key.into();
                let value = value.into();
                if key.trim().is_empty() || value.trim().is_empty() {
                    return self;
                }
                let mut map = self.metadata.unwrap_or_default();
                map.insert(key, value);
                self.metadata = Some(map);
                self
            }

            /// Set the response format (e.g., JSON schema for structured outputs).
            pub fn response_format(mut self, response_format: ResponseFormat) -> Self {
                self.response_format = Some(response_format);
                self
            }

            /// Set stop sequences.
            pub fn stop(mut self, stop: Vec<String>) -> Self {
                self.stop = Some(stop);
                self
            }

            /// Set tools available for the model to call.
            pub fn tools(mut self, tools: Vec<crate::types::Tool>) -> Self {
                self.tools = Some(tools);
                self
            }

            /// Set the tool choice strategy.
            pub fn tool_choice(mut self, tool_choice: crate::types::ToolChoice) -> Self {
                self.tool_choice = Some(tool_choice);
                self
            }

            /// Set a request ID for tracing.
            pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
                self.request_id = Some(request_id.into());
                self
            }

            /// Add a custom header to the request.
            pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
                self.headers.push((key.into(), value.into()));
                self
            }

            /// Set the request timeout.
            pub fn timeout(mut self, timeout: Duration) -> Self {
                self.timeout = Some(timeout);
                self
            }

            /// Set retry configuration.
            pub fn retry(mut self, retry: RetryConfig) -> Self {
                self.retry = Some(retry);
                self
            }
        }
    };
}

/// Builder for LLM proxy chat requests (async + streaming).
#[derive(Clone, Debug, Default)]
pub struct ChatRequestBuilder {
    pub(crate) model: Option<Model>,
    pub(crate) max_tokens: Option<i64>,
    pub(crate) temperature: Option<f64>,
    pub(crate) messages: Vec<ProxyMessage>,
    pub(crate) metadata: Option<HashMap<String, String>>,
    pub(crate) response_format: Option<ResponseFormat>,
    pub(crate) stop: Option<Vec<String>>,
    pub(crate) tools: Option<Vec<crate::types::Tool>>,
    pub(crate) tool_choice: Option<crate::types::ToolChoice>,
    pub(crate) request_id: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) retry: Option<RetryConfig>,
}

// Generate shared builder methods for ChatRequestBuilder
impl_chat_builder_common!(ChatRequestBuilder);

impl ChatRequestBuilder {
    /// Create a new chat request builder for the given model.
    pub fn new(model: impl Into<Model>) -> Self {
        Self {
            model: Some(model.into()),
            ..Default::default()
        }
    }

    fn build_options(&self) -> ProxyOptions {
        let mut opts = ProxyOptions::default();
        if let Some(req_id) = &self.request_id {
            opts = opts.with_request_id(req_id.clone());
        }
        for (k, v) in &self.headers {
            opts = opts.with_header(k.clone(), v.clone());
        }
        if let Some(timeout) = self.timeout {
            opts = opts.with_timeout(timeout);
        }
        if let Some(retry) = &self.retry {
            opts = opts.with_retry(retry.clone());
        }
        opts
    }

    pub fn build_request(&self) -> Result<ProxyRequest> {
        let model = self
            .model
            .clone()
            .ok_or_else(|| Error::Validation("model is required".into()))?;

        if self.messages.is_empty() {
            return Err(Error::Validation(
                ValidationError::new("at least one message is required").with_field("messages"),
            ));
        }
        if !self
            .messages
            .iter()
            .any(|msg| msg.role == crate::types::MessageRole::User)
        {
            return Err(Error::Validation(
                ValidationError::new("at least one user message is required")
                    .with_field("messages"),
            ));
        }

        let req = ProxyRequest {
            model,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            messages: self.messages.clone(),
            metadata: self.metadata.clone(),
            response_format: self.response_format.clone(),
            stop: self.stop.clone(),
            tools: self.tools.clone(),
            tool_choice: self.tool_choice.clone(),
        };
        req.validate()?;
        Ok(req)
    }

    /// Execute the chat request (non-streaming, async).
    #[cfg(feature = "client")]
    pub async fn send(self, client: &LLMClient) -> Result<ProxyResponse> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy(req, opts).await
    }

    /// Execute the chat request and stream responses (async).
    #[cfg(all(feature = "client", feature = "streaming"))]
    pub async fn stream(self, client: &LLMClient) -> Result<StreamHandle> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy_stream(req, opts).await
    }

    /// Execute the chat request and stream text deltas (async).
    #[cfg(all(feature = "client", feature = "streaming"))]
    pub async fn stream_deltas(
        self,
        client: &LLMClient,
    ) -> Result<std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<String>> + Send>>> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy_stream_deltas(req, opts).await
    }

    /// Create a structured output builder with automatic schema generation.
    ///
    /// This method transitions the builder to a [`StructuredChatBuilder`] that
    /// automatically generates a JSON schema from the type `T` and handles
    /// validation retries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use schemars::JsonSchema;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    /// struct Person {
    ///     name: String,
    ///     age: u32,
    /// }
    ///
    /// let result = client.chat()
    ///     .model("claude-sonnet-4-20250514")
    ///     .user("Extract: John Doe, 30 years old")
    ///     .structured::<Person>()
    ///     .max_retries(2)
    ///     .send()
    ///     .await?;
    /// ```
    #[cfg(feature = "client")]
    pub fn structured<T>(self) -> crate::structured::StructuredChatBuilder<T>
    where
        T: JsonSchema + DeserializeOwned,
    {
        crate::structured::StructuredChatBuilder::new(self)
    }

    /// Execute the chat request and stream structured JSON payloads (async).
    ///
    /// The request must include a structured response_format (type=json_schema),
    /// and uses NDJSON framing per the /llm/proxy structured streaming contract.
    #[cfg(all(feature = "client", feature = "streaming"))]
    pub async fn stream_json<T>(self, client: &LLMClient) -> Result<StructuredJSONStream<T>>
    where
        T: DeserializeOwned,
    {
        let req = self.build_request()?;
        match &req.response_format {
            Some(format) if format.is_structured() => {}
            Some(_) => {
                return Err(Error::Validation(
                    ValidationError::new("response_format must be structured (type=json_schema)")
                        .with_field("response_format.type"),
                ));
            }
            None => {
                return Err(Error::Validation(
                    ValidationError::new("response_format is required for structured streaming")
                        .with_field("response_format"),
                ));
            }
        }
        let opts = self.build_options();
        let stream = client.proxy_stream(req, opts).await?;
        Ok(StructuredJSONStream::new(stream))
    }

    /// Execute the chat request (blocking).
    #[cfg(feature = "blocking")]
    pub fn send_blocking(self, client: &BlockingLLMClient) -> Result<ProxyResponse> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy(req, opts)
    }

    /// Execute the chat request and stream responses (blocking).
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_blocking(self, client: &BlockingLLMClient) -> Result<BlockingProxyHandle> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy_stream(req, opts)
    }

    /// Execute the chat request and stream text deltas (blocking).
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_deltas_blocking(
        self,
        client: &BlockingLLMClient,
    ) -> Result<Box<dyn Iterator<Item = Result<String>>>> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy_stream_deltas(req, opts)
    }

    /// Execute the chat request and stream structured JSON payloads (blocking).
    ///
    /// The request must include a structured response_format (type=json_schema),
    /// and uses NDJSON framing per the /llm/proxy structured streaming contract.
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_json_blocking<T>(
        self,
        client: &BlockingLLMClient,
    ) -> Result<BlockingStructuredJSONStream<T>>
    where
        T: DeserializeOwned,
    {
        let req = self.build_request()?;
        match &req.response_format {
            Some(format) if format.is_structured() => {}
            Some(_) => {
                return Err(Error::Validation(
                    ValidationError::new("response_format must be structured (type=json_schema)")
                        .with_field("response_format.type"),
                ));
            }
            None => {
                return Err(Error::Validation(
                    ValidationError::new("response_format is required for structured streaming")
                        .with_field("response_format"),
                ));
            }
        }
        let opts = self.build_options();
        let stream = client.proxy_stream(req, opts)?;
        Ok(BlockingStructuredJSONStream::new(stream))
    }
}

/// Header name for customer ID attribution.
pub const CUSTOMER_ID_HEADER: &str = "X-ModelRelay-Customer-Id";

/// Builder for customer-attributed LLM proxy chat requests.
///
/// Unlike [`ChatRequestBuilder`], this builder does not require a model since
/// the customer's tier determines which model to use. Create via
/// [`LLMClient::for_customer`].
#[derive(Clone, Debug, Default)]
pub struct CustomerChatRequestBuilder {
    pub(crate) customer_id: String,
    pub(crate) max_tokens: Option<i64>,
    pub(crate) temperature: Option<f64>,
    pub(crate) messages: Vec<ProxyMessage>,
    pub(crate) metadata: Option<HashMap<String, String>>,
    pub(crate) response_format: Option<ResponseFormat>,
    pub(crate) stop: Option<Vec<String>>,
    pub(crate) tools: Option<Vec<crate::types::Tool>>,
    pub(crate) tool_choice: Option<crate::types::ToolChoice>,
    pub(crate) request_id: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) retry: Option<RetryConfig>,
}

// Generate shared builder methods for CustomerChatRequestBuilder
impl_chat_builder_common!(CustomerChatRequestBuilder);

impl CustomerChatRequestBuilder {
    /// Create a new customer chat builder for the given customer ID.
    pub fn new(customer_id: impl Into<String>) -> Self {
        Self {
            customer_id: customer_id.into(),
            ..Default::default()
        }
    }

    fn build_options(&self) -> ProxyOptions {
        let mut opts = ProxyOptions::default();
        if let Some(req_id) = &self.request_id {
            opts = opts.with_request_id(req_id.clone());
        }
        // Customer ID is passed directly to proxy_customer/proxy_customer_stream
        for (k, v) in &self.headers {
            opts = opts.with_header(k.clone(), v.clone());
        }
        if let Some(timeout) = self.timeout {
            opts = opts.with_timeout(timeout);
        }
        if let Some(retry) = &self.retry {
            opts = opts.with_retry(retry.clone());
        }
        opts
    }

    /// Build the request body. Uses an empty model since the tier determines it.
    pub(crate) fn build_request_body(&self) -> Result<CustomerProxyRequestBody> {
        if self.messages.is_empty() {
            return Err(Error::Validation(
                crate::errors::ValidationError::new("at least one message is required")
                    .with_field("messages"),
            ));
        }
        if !self
            .messages
            .iter()
            .any(|msg| msg.role == crate::types::MessageRole::User)
        {
            return Err(Error::Validation(
                crate::errors::ValidationError::new("at least one user message is required")
                    .with_field("messages"),
            ));
        }
        Ok(CustomerProxyRequestBody {
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            messages: self.messages.clone(),
            metadata: self.metadata.clone(),
            response_format: self.response_format.clone(),
            stop: self.stop.clone(),
        })
    }

    /// Execute the chat request (non-streaming, async).
    #[cfg(feature = "client")]
    pub async fn send(self, client: &LLMClient) -> Result<ProxyResponse> {
        let body = self.build_request_body()?;
        let opts = self.build_options();
        client.proxy_customer(&self.customer_id, body, opts).await
    }

    /// Execute the chat request and stream responses (async).
    #[cfg(all(feature = "client", feature = "streaming"))]
    pub async fn stream(self, client: &LLMClient) -> Result<StreamHandle> {
        let body = self.build_request_body()?;
        let opts = self.build_options();
        client
            .proxy_customer_stream(&self.customer_id, body, opts)
            .await
    }

    /// Execute the chat request (blocking).
    #[cfg(feature = "blocking")]
    pub fn send_blocking(self, client: &BlockingLLMClient) -> Result<ProxyResponse> {
        let body = self.build_request_body()?;
        let opts = self.build_options();
        client.proxy_customer(&self.customer_id, body, opts)
    }

    /// Execute the chat request and stream responses (blocking).
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_blocking(self, client: &BlockingLLMClient) -> Result<BlockingProxyHandle> {
        let body = self.build_request_body()?;
        let opts = self.build_options();
        client.proxy_customer_stream(&self.customer_id, body, opts)
    }

    /// Execute the chat request and stream structured JSON payloads (async).
    ///
    /// The request must include a structured response_format (type=json_schema),
    /// and uses NDJSON framing per the /llm/proxy structured streaming contract.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use serde::Deserialize;
    /// use modelrelay::CustomerChatRequestBuilder;
    ///
    /// #[derive(Debug, Deserialize)]
    /// struct CommitMessage {
    ///     title: String,
    ///     body: Option<String>,
    /// }
    ///
    /// let stream = CustomerChatRequestBuilder::new("user-123")
    ///     .user("Generate a commit message for: ...")
    ///     .response_format(ResponseFormat::json_schema::<CommitMessage>("CommitMessage"))
    ///     .stream_json::<CommitMessage>(&client.llm())
    ///     .await?;
    ///
    /// let result = stream.collect().await?;
    /// println!("Title: {}", result.title);
    /// ```
    #[cfg(all(feature = "client", feature = "streaming"))]
    pub async fn stream_json<T>(self, client: &LLMClient) -> Result<StructuredJSONStream<T>>
    where
        T: DeserializeOwned,
    {
        let body = self.build_request_body()?;
        match &body.response_format {
            Some(format) if format.is_structured() => {}
            Some(_) => {
                return Err(Error::Validation(
                    ValidationError::new("response_format must be structured (type=json_schema)")
                        .with_field("response_format.type"),
                ));
            }
            None => {
                return Err(Error::Validation(
                    ValidationError::new("response_format is required for structured streaming")
                        .with_field("response_format"),
                ));
            }
        }
        let opts = self.build_options();
        let stream = client
            .proxy_customer_stream(&self.customer_id, body, opts)
            .await?;
        Ok(StructuredJSONStream::new(stream))
    }

    /// Execute the chat request and stream structured JSON payloads (blocking).
    ///
    /// The request must include a structured response_format (type=json_schema),
    /// and uses NDJSON framing per the /llm/proxy structured streaming contract.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use modelrelay::{CustomerChatRequestBuilder, ResponseFormat};
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Deserialize)]
    /// struct CommitMessage {
    ///     title: String,
    ///     body: Option<String>,
    /// }
    ///
    /// let mut stream = CustomerChatRequestBuilder::new("user-123")
    ///     .user("Generate a commit message for: ...")
    ///     .response_format(ResponseFormat::json_schema::<CommitMessage>("CommitMessage"))
    ///     .stream_json_blocking::<CommitMessage>(&client.llm())?;
    ///
    /// let result = stream.collect()?;
    /// println!("Title: {}", result.title);
    /// ```
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_json_blocking<T>(
        self,
        client: &BlockingLLMClient,
    ) -> Result<BlockingStructuredJSONStream<T>>
    where
        T: DeserializeOwned,
    {
        let body = self.build_request_body()?;
        match &body.response_format {
            Some(format) if format.is_structured() => {}
            Some(_) => {
                return Err(Error::Validation(
                    ValidationError::new("response_format must be structured (type=json_schema)")
                        .with_field("response_format.type"),
                ));
            }
            None => {
                return Err(Error::Validation(
                    ValidationError::new("response_format is required for structured streaming")
                        .with_field("response_format"),
                ));
            }
        }
        let opts = self.build_options();
        let stream = client.proxy_customer_stream(&self.customer_id, body, opts)?;
        Ok(BlockingStructuredJSONStream::new(stream))
    }
}

/// Request body for customer-attributed proxy requests (no model field).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CustomerProxyRequestBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub messages: Vec<ProxyMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

/// Thin adapter over streaming events to yield text deltas and final metadata.
#[cfg(feature = "streaming")]
#[derive(Debug)]
pub struct ChatStreamAdapter<S> {
    inner: S,
    finished: bool,
    final_usage: Option<Usage>,
    final_stop_reason: Option<StopReason>,
    final_request_id: Option<String>,
}

#[cfg(all(feature = "client", feature = "streaming"))]
impl ChatStreamAdapter<StreamHandle> {
    pub fn new(stream: StreamHandle) -> Self {
        Self {
            inner: stream,
            finished: false,
            final_usage: None,
            final_stop_reason: None,
            final_request_id: None,
        }
    }

    /// Pull the next text delta (if any) and track final usage/stop metadata.
    pub async fn next_delta(&mut self) -> Result<Option<String>> {
        use futures_util::StreamExt;

        while let Some(item) = self.inner.next().await {
            let evt = item?;
            match evt.kind {
                StreamEventKind::MessageDelta => {
                    if let Some(delta) = evt.text_delta {
                        return Ok(Some(delta));
                    }
                }
                StreamEventKind::MessageStop => {
                    self.finished = true;
                    self.final_usage = evt.usage;
                    self.final_stop_reason = evt.stop_reason;
                    self.final_request_id = evt
                        .request_id
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Final usage info if the stream finished.
    pub fn final_usage(&self) -> Option<&Usage> {
        self.final_usage.as_ref()
    }

    /// Final stop reason if the stream finished.
    pub fn final_stop_reason(&self) -> Option<&StopReason> {
        self.final_stop_reason.as_ref()
    }

    /// Final request id if known.
    pub fn final_request_id(&self) -> Option<&str> {
        self.final_request_id.as_deref()
    }

    /// Convert to a stream of deltas, propagating errors and tracking final state.
    pub fn into_stream(self) -> impl futures_core::Stream<Item = Result<String>> {
        stream::unfold(self, |mut adapter| async move {
            match adapter.next_delta().await {
                Ok(Some(delta)) => Some((Ok(delta), adapter)),
                Ok(None) => None,
                Err(err) => Some((Err(err), adapter)),
            }
        })
    }
}

/// Structured streaming record kinds surfaced by the helper.
#[cfg(feature = "streaming")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredRecordKind {
    Update,
    Completion,
}

/// Typed structured JSON event yielded from the NDJSON stream.
#[cfg(feature = "streaming")]
#[derive(Debug, Clone)]
pub struct StructuredJSONEvent<T> {
    pub kind: StructuredRecordKind,
    pub payload: T,
    pub request_id: Option<String>,
    /// Set of field paths that are complete (have their closing delimiter).
    /// Use dot notation for nested fields (e.g., "metadata.author").
    /// Check with complete_fields.contains("fieldName").
    pub complete_fields: std::collections::HashSet<String>,
}

/// Helper over NDJSON streaming events to yield structured JSON payloads.
#[cfg(all(feature = "client", feature = "streaming"))]
pub struct StructuredJSONStream<T> {
    inner: StreamHandle,
    finished: bool,
    saw_completion: bool,
    _marker: std::marker::PhantomData<T>,
}

#[cfg(all(feature = "client", feature = "streaming"))]
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

    /// Pull the next structured JSON event, skipping start/unknown records.
    pub async fn next(&mut self) -> Result<Option<StructuredJSONEvent<T>>> {
        use futures_util::StreamExt;

        if self.finished {
            return Ok(None);
        }

        while let Some(item) = self.inner.next().await {
            let evt = item?;
            let value = match evt.data {
                Some(ref v) if v.is_object() => v,
                _ => continue,
            };
            let record_type = value
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_lowercase())
                .unwrap_or_default();

            match record_type.as_str() {
                "" | "start" => continue,
                "update" | "completion" => {
                    let payload_value = value.get("payload").cloned().ok_or_else(|| {
                        Error::Transport(TransportError {
                            kind: TransportErrorKind::Request,
                            message: "structured stream record missing payload".to_string(),
                            source: None,
                            retries: None,
                        })
                    })?;
                    let payload: T =
                        serde_json::from_value(payload_value).map_err(Error::Serialization)?;
                    let kind = if record_type == "update" {
                        StructuredRecordKind::Update
                    } else {
                        self.saw_completion = true;
                        StructuredRecordKind::Completion
                    };
                    let request_id = evt
                        .request_id
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    // Extract complete_fields array and convert to HashSet
                    let complete_fields: std::collections::HashSet<String> = value
                        .get("complete_fields")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    return Ok(Some(StructuredJSONEvent {
                        kind,
                        payload,
                        request_id,
                        complete_fields,
                    }));
                }
                "error" => {
                    self.saw_completion = true;
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
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    return Err(APIError {
                        status,
                        code,
                        message,
                        request_id,
                        fields: Vec::new(),
                        retries: None,
                        raw_body: None,
                    }
                    .into());
                }
                _ => continue,
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

    /// Drain the stream and return the final structured payload from the completion record.
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

    /// Request identifier returned by the server (if any).
    pub fn request_id(&self) -> Option<&str> {
        self.inner.request_id()
    }
}

/// Blocking helper over NDJSON streaming events to yield structured JSON payloads.
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub struct BlockingStructuredJSONStream<T> {
    inner: BlockingProxyHandle,
    finished: bool,
    saw_completion: bool,
    _marker: std::marker::PhantomData<T>,
}

#[cfg(all(feature = "blocking", feature = "streaming"))]
impl<T> BlockingStructuredJSONStream<T>
where
    T: DeserializeOwned,
{
    pub fn new(stream: BlockingProxyHandle) -> Self {
        Self {
            inner: stream,
            finished: false,
            saw_completion: false,
            _marker: std::marker::PhantomData,
        }
    }

    /// Pull the next structured JSON event, skipping start/unknown records.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<StructuredJSONEvent<T>>> {
        if self.finished {
            return Ok(None);
        }

        while let Some(evt) = self.inner.next()? {
            let value = match evt.data {
                Some(ref v) if v.is_object() => v,
                _ => continue,
            };
            let record_type = value
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_lowercase())
                .unwrap_or_default();

            match record_type.as_str() {
                "" | "start" => continue,
                "update" | "completion" => {
                    let payload_value = value.get("payload").cloned().ok_or_else(|| {
                        Error::Transport(TransportError {
                            kind: TransportErrorKind::Request,
                            message: "structured stream record missing payload".to_string(),
                            source: None,
                            retries: None,
                        })
                    })?;
                    let payload: T =
                        serde_json::from_value(payload_value).map_err(Error::Serialization)?;
                    let kind = if record_type == "update" {
                        StructuredRecordKind::Update
                    } else {
                        self.saw_completion = true;
                        StructuredRecordKind::Completion
                    };
                    let request_id = evt
                        .request_id
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    let complete_fields: std::collections::HashSet<String> = value
                        .get("complete_fields")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    return Ok(Some(StructuredJSONEvent {
                        kind,
                        payload,
                        request_id,
                        complete_fields,
                    }));
                }
                "error" => {
                    self.saw_completion = true;
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
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    return Err(APIError {
                        status,
                        code,
                        message,
                        request_id,
                        fields: Vec::new(),
                        retries: None,
                        raw_body: None,
                    }
                    .into());
                }
                _ => continue,
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

    /// Drain the stream and return the final structured payload from the completion record.
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

    /// Request identifier returned by the server (if any).
    pub fn request_id(&self) -> Option<&str> {
        self.inner.request_id()
    }
}

/// Blocking streaming adapter.
#[cfg(all(feature = "blocking", feature = "streaming"))]
impl ChatStreamAdapter<BlockingProxyHandle> {
    pub fn new(stream: BlockingProxyHandle) -> Self {
        Self {
            inner: stream,
            finished: false,
            final_usage: None,
            final_stop_reason: None,
            final_request_id: None,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.inner.request_id()
    }

    pub fn next_delta(&mut self) -> Result<Option<String>> {
        while let Some(evt) = self.inner.next()? {
            match evt.kind {
                StreamEventKind::MessageDelta => {
                    if let Some(delta) = evt.text_delta {
                        return Ok(Some(delta));
                    }
                }
                StreamEventKind::MessageStop => {
                    self.finished = true;
                    self.final_usage = evt.usage;
                    self.final_stop_reason = evt.stop_reason;
                    self.final_request_id = evt
                        .request_id
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub fn final_usage(&self) -> Option<&Usage> {
        self.final_usage.as_ref()
    }

    pub fn final_stop_reason(&self) -> Option<&StopReason> {
        self.final_stop_reason.as_ref()
    }

    pub fn final_request_id(&self) -> Option<&str> {
        self.final_request_id.as_deref()
    }

    /// Iterate over text deltas until completion or error.
    #[allow(clippy::should_implement_trait)]
    pub fn into_iter(self) -> impl Iterator<Item = Result<String>> {
        let mut adapter = self;
        std::iter::from_fn(move || match adapter.next_delta() {
            Ok(Some(delta)) => Some(Ok(delta)),
            Ok(None) => None,
            Err(err) => Some(Err(err)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Model, ResponseFormatKind, StreamEvent, StreamEventKind};
    use crate::ClientBuilder;

    #[test]
    fn build_request_requires_user_message() {
        let builder = ChatRequestBuilder::new(Model::from("gpt-4o-mini")).system("just a system");
        let err = builder.build_request().unwrap_err();
        match err {
            Error::Validation(msg) => {
                assert!(
                    msg.to_string().contains("user"),
                    "unexpected validation: {msg}"
                );
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn metadata_entry_ignores_empty_pairs() {
        let req = ChatRequestBuilder::new(Model::from("gpt-4o-mini"))
            .user("hello")
            .metadata_entry("trace_id", "abc123")
            .metadata_entry("", "should_skip")
            .metadata_entry("empty", "")
            .build_request()
            .unwrap();
        let meta = req.metadata.unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta.get("trace_id"), Some(&"abc123".to_string()));
    }

    #[test]
    fn role_helpers_append_expected_roles() {
        use crate::types::MessageRole;
        let req = ChatRequestBuilder::new("gpt-4o-mini")
            .system("sys")
            .user("u1")
            .assistant("a1")
            .build_request()
            .unwrap();
        let roles: Vec<_> = req.messages.iter().map(|m| m.role).collect();
        assert_eq!(
            roles,
            vec![
                MessageRole::System,
                MessageRole::User,
                MessageRole::Assistant
            ]
        );
    }

    #[cfg(all(feature = "client", feature = "streaming"))]
    #[tokio::test]
    async fn stream_json_requires_structured_response_format() {
        let client = ClientBuilder::new()
            .api_key("mr_sk_test")
            .build()
            .expect("client build");

        // Missing response_format
        let builder = ChatRequestBuilder::new(Model::from("gpt-4o-mini")).user("hi");
        let result = builder
            .clone()
            .stream_json::<serde_json::Value>(&client.llm())
            .await;
        match result {
            Err(Error::Validation(v)) => {
                assert!(
                    v.to_string().contains("response_format"),
                    "unexpected validation error: {v}"
                );
            }
            Ok(_) => panic!("expected Validation error, got Ok"),
            Err(other) => panic!("expected Validation error, got {other:?}"),
        }

        // Non-structured response_format (Text)
        let format = ResponseFormat {
            kind: ResponseFormatKind::Text,
            json_schema: None,
        };
        let builder = ChatRequestBuilder::new(Model::from("gpt-4o-mini"))
            .user("hi")
            .response_format(format);
        let result = builder
            .stream_json::<serde_json::Value>(&client.llm())
            .await;
        match result {
            Err(Error::Validation(v)) => {
                assert!(
                    v.to_string().contains("response_format must be structured"),
                    "unexpected validation error: {v}"
                );
            }
            Ok(_) => panic!("expected Validation error, got Ok"),
            Err(other) => panic!("expected Validation error, got {other:?}"),
        }
    }

    #[cfg(all(feature = "client", feature = "streaming"))]
    #[tokio::test]
    async fn structured_json_stream_yields_update_and_completion() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Item {
            id: String,
        }

        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct ItemsPayload {
            items: Vec<Item>,
        }

        let events = vec![
            StreamEvent {
                kind: StreamEventKind::Custom,
                event: "structured".into(),
                data: Some(serde_json::json!({"type":"start","request_id":"tiers-1"})),
                text_delta: None,
                tool_call_delta: None,
                tool_calls: None,
                response_id: None,
                model: None,
                stop_reason: None,
                usage: None,
                request_id: None,
                raw: String::new(),
            },
            StreamEvent {
                kind: StreamEventKind::Custom,
                event: "structured".into(),
                data: Some(serde_json::json!({"type":"update","payload":{"items":[{"id":"one"}]}})),
                text_delta: None,
                tool_call_delta: None,
                tool_calls: None,
                response_id: None,
                model: None,
                stop_reason: None,
                usage: None,
                request_id: None,
                raw: String::new(),
            },
            StreamEvent {
                kind: StreamEventKind::Custom,
                event: "structured".into(),
                data: Some(
                    serde_json::json!({"type":"completion","payload":{"items":[{"id":"one"},{"id":"two"}]}}),
                ),
                text_delta: None,
                tool_call_delta: None,
                tool_calls: None,
                response_id: None,
                model: None,
                stop_reason: None,
                usage: None,
                request_id: None,
                raw: String::new(),
            },
        ];

        let handle = StreamHandle::from_events_with_request_id(
            events.clone(),
            Some("req-structured".into()),
        );
        let mut stream = StructuredJSONStream::<ItemsPayload>::new(handle);

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.kind, StructuredRecordKind::Update);
        assert_eq!(first.payload.items.len(), 1);
        assert_eq!(first.payload.items[0].id, "one");

        let second = stream.next().await.unwrap().unwrap();
        assert_eq!(second.kind, StructuredRecordKind::Completion);
        assert_eq!(second.payload.items.len(), 2);
        assert_eq!(second.request_id.as_deref(), Some("req-structured"));

        let handle2 =
            StreamHandle::from_events_with_request_id(events, Some("req-structured".into()));
        let stream2 = StructuredJSONStream::<ItemsPayload>::new(handle2);
        let collected = stream2.collect().await.unwrap();
        assert_eq!(collected.items.len(), 2);
    }

    #[cfg(all(feature = "client", feature = "streaming"))]
    #[tokio::test]
    async fn structured_json_stream_maps_error_and_protocol_violation() {
        // Error record surfaces as APIError.
        let error_events = vec![StreamEvent {
            kind: StreamEventKind::Custom,
            event: "structured".into(),
            data: Some(
                serde_json::json!({"type":"error","code":"SERVICE_UNAVAILABLE","message":"upstream timeout","status":502}),
            ),
            text_delta: None,
            tool_call_delta: None,
            tool_calls: None,
            response_id: None,
            model: None,
            stop_reason: None,
            usage: None,
            request_id: None,
            raw: String::new(),
        }];
        let handle_err =
            StreamHandle::from_events_with_request_id(error_events, Some("req-error".into()));
        let mut err_stream = StructuredJSONStream::<serde_json::Value>::new(handle_err);
        let err = err_stream.next().await.unwrap_err();
        match err {
            Error::Api(api) => {
                assert_eq!(api.status, 502);
                assert_eq!(api.code.as_deref(), Some("SERVICE_UNAVAILABLE"));
                assert_eq!(api.request_id.as_deref(), Some("req-error"));
            }
            other => panic!("expected API error, got {other:?}"),
        }

        // Stream ending without completion/error becomes a transport error.
        let update_only = vec![StreamEvent {
            kind: StreamEventKind::Custom,
            event: "structured".into(),
            data: Some(serde_json::json!({"type":"update","payload":{"items":[{"id":"one"}]}})),
            text_delta: None,
            tool_call_delta: None,
            tool_calls: None,
            response_id: None,
            model: None,
            stop_reason: None,
            usage: None,
            request_id: None,
            raw: String::new(),
        }];
        let handle_proto =
            StreamHandle::from_events_with_request_id(update_only, Some("req-incomplete".into()));
        let stream_proto = StructuredJSONStream::<serde_json::Value>::new(handle_proto);
        let err = stream_proto.collect().await.unwrap_err();
        match err {
            Error::Transport(te) => {
                assert!(
                    te.message
                        .contains("structured stream ended without completion or error"),
                    "unexpected message: {}",
                    te.message
                );
            }
            other => panic!("expected Transport error, got {other:?}"),
        }
    }

    #[cfg(all(feature = "client", feature = "streaming"))]
    #[tokio::test]
    async fn customer_stream_json_requires_structured_response_format() {
        let client = ClientBuilder::new()
            .api_key("mr_sk_test")
            .build()
            .expect("client build");

        // Missing response_format
        let builder = CustomerChatRequestBuilder::new("customer-123").user("hi");
        let result = builder
            .clone()
            .stream_json::<serde_json::Value>(&client.llm())
            .await;
        match result {
            Err(Error::Validation(v)) => {
                assert!(
                    v.to_string().contains("response_format"),
                    "unexpected validation error: {v}"
                );
            }
            Ok(_) => panic!("expected Validation error, got Ok"),
            Err(other) => panic!("expected Validation error, got {other:?}"),
        }

        // Non-structured response_format (Text)
        let format = ResponseFormat {
            kind: ResponseFormatKind::Text,
            json_schema: None,
        };
        let builder = CustomerChatRequestBuilder::new("customer-123")
            .user("hi")
            .response_format(format);
        let result = builder
            .stream_json::<serde_json::Value>(&client.llm())
            .await;
        match result {
            Err(Error::Validation(v)) => {
                assert!(
                    v.to_string().contains("response_format must be structured"),
                    "unexpected validation error: {v}"
                );
            }
            Ok(_) => panic!("expected Validation error, got Ok"),
            Err(other) => panic!("expected Validation error, got {other:?}"),
        }
    }

    #[test]
    fn customer_build_request_body_requires_user_message() {
        let builder = CustomerChatRequestBuilder::new("customer-123").system("just a system");
        let err = builder.build_request_body().unwrap_err();
        match err {
            Error::Validation(msg) => {
                assert!(
                    msg.to_string().contains("user"),
                    "unexpected validation: {msg}"
                );
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn customer_metadata_entry_ignores_empty_pairs() {
        let body = CustomerChatRequestBuilder::new("customer-123")
            .user("hello")
            .metadata_entry("trace_id", "abc123")
            .metadata_entry("", "should_skip")
            .metadata_entry("empty", "")
            .build_request_body()
            .unwrap();
        let meta = body.metadata.unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta.get("trace_id"), Some(&"abc123".to_string()));
    }
}
