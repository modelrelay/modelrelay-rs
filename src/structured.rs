//! Ergonomic structured output API with schema inference and validation.
//!
//! This module provides type-safe structured outputs using [`schemars`] for automatic
//! JSON schema generation. The API handles schema construction, validation retries
//! with error feedback, and strongly-typed result parsing.
//!
//! # Example
//!
//! ```ignore
//! use modelrelay::ResponseBuilder;
//! use schemars::JsonSchema;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize, JsonSchema)]
//! struct Person {
//!     name: String,
//!     age: u32,
//! }
//!
//! let result = ResponseBuilder::new()
//!     .model("claude-sonnet-4-20250514")
//!     .user("Extract: John Doe, 30 years old")
//!     .structured::<Person>()
//!     .max_retries(2)
//!     .send(&client.responses())
//!     .await?;
//!
//! println!("Name: {}, Age: {}", result.value.name, result.value.age);
//! ```

use std::marker::PhantomData;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

#[cfg(feature = "blocking")]
use crate::blocking::BlockingResponsesClient;
#[cfg(all(feature = "blocking", feature = "streaming"))]
use crate::responses::BlockingStructuredJSONStream;
use crate::types::{
    ContentPart, InputItem, JSONSchemaFormat, MessageRole, OutputFormat, OutputFormatKind,
    OutputItem, Response,
};

// ============================================================================
// Error Types
// ============================================================================

/// Record of a single structured output attempt.
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    /// Which attempt (1-based).
    pub attempt: u32,
    /// Raw JSON returned by the model.
    pub raw_json: String,
    /// The error that occurred.
    pub error: StructuredErrorKind,
}

/// Specific kind of structured output error.
#[derive(Debug, Clone)]
pub enum StructuredErrorKind {
    /// JSON decode/parse error.
    Decode {
        /// The decode error message.
        message: String,
    },
    /// Schema validation error (type mismatch, missing fields, etc.).
    Validation {
        /// Field-level validation issues.
        issues: Vec<ValidationIssue>,
    },
}

impl std::fmt::Display for StructuredErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StructuredErrorKind::Decode { message } => write!(f, "decode error: {}", message),
            StructuredErrorKind::Validation { issues } => {
                write!(f, "validation error: ")?;
                for (i, issue) in issues.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{}", issue)?;
                }
                Ok(())
            }
        }
    }
}

/// A single field-level validation issue.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// JSON path to the problematic field (e.g., "person.address.city").
    pub path: Option<String>,
    /// Description of the issue.
    pub message: String,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{}: {}", path, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

/// Error returned when structured output fails on first attempt (before retries).
#[derive(Debug)]
pub struct StructuredDecodeError {
    /// The raw JSON that failed to decode.
    pub raw_json: String,
    /// The decode error message.
    pub message: String,
    /// Which attempt this was (1-based).
    pub attempt: u32,
}

impl std::fmt::Display for StructuredDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "structured output decode error (attempt {}): {}",
            self.attempt, self.message
        )
    }
}

impl std::error::Error for StructuredDecodeError {}

/// Error returned when all retry attempts are exhausted.
#[derive(Debug)]
pub struct StructuredExhaustedError {
    /// Raw JSON from the last attempt.
    pub last_raw_json: String,
    /// History of all attempts.
    pub all_attempts: Vec<AttemptRecord>,
    /// The final error that caused exhaustion.
    pub final_error: StructuredErrorKind,
}

impl std::fmt::Display for StructuredExhaustedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "structured output failed after {} attempts: {}",
            self.all_attempts.len(),
            self.final_error
        )
    }
}

impl std::error::Error for StructuredExhaustedError {}

/// Unified error type for structured output operations.
#[derive(Debug)]
pub enum StructuredError {
    /// Decode/parse failure.
    Decode(StructuredDecodeError),
    /// All retries exhausted.
    Exhausted(StructuredExhaustedError),
    /// Underlying SDK error (transport, API, etc.).
    Sdk(crate::Error),
}

impl std::fmt::Display for StructuredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StructuredError::Decode(e) => write!(f, "{}", e),
            StructuredError::Exhausted(e) => write!(f, "{}", e),
            StructuredError::Sdk(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for StructuredError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StructuredError::Decode(e) => Some(e),
            StructuredError::Exhausted(e) => Some(e),
            StructuredError::Sdk(e) => Some(e),
        }
    }
}

impl From<crate::Error> for StructuredError {
    fn from(e: crate::Error) -> Self {
        StructuredError::Sdk(e)
    }
}

// ============================================================================
// Retry Handler
// ============================================================================

/// Handler for customizing retry behavior on validation failures.
///
/// Implement this trait to customize how retry messages are constructed
/// when validation fails. The default implementation appends a simple
/// error message asking the model to correct its output.
pub trait RetryHandler: Send + Sync {
    /// Called when validation fails. Returns messages to append to the conversation
    /// for the retry, or `None` to stop retrying immediately.
    ///
    /// # Arguments
    /// * `attempt` - Current attempt number (1-based)
    /// * `raw_json` - The raw JSON that failed validation
    /// * `error` - The validation/decode error that occurred
    /// * `messages` - The original conversation messages
    fn on_validation_error(
        &self,
        attempt: u32,
        raw_json: &str,
        error: &StructuredErrorKind,
        messages: &[InputItem],
    ) -> Option<Vec<InputItem>>;
}

/// Default retry handler that appends a simple error correction message.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultRetryHandler;

impl RetryHandler for DefaultRetryHandler {
    fn on_validation_error(
        &self,
        _attempt: u32,
        _raw_json: &str,
        error: &StructuredErrorKind,
        _messages: &[InputItem],
    ) -> Option<Vec<InputItem>> {
        let error_msg = format!(
            "The previous response did not match the expected schema. Error: {}. \
             Please provide a response that matches the schema exactly.",
            error
        );
        Some(vec![InputItem::user(error_msg)])
    }
}

// ============================================================================
// Options and Result
// ============================================================================

/// Options for structured output requests.
#[derive(Debug, Clone)]
pub struct StructuredOptions<H: RetryHandler = DefaultRetryHandler> {
    /// Maximum number of retry attempts on validation failure (default: 0).
    pub max_retries: u32,
    /// Handler for customizing retry messages.
    pub retry_handler: H,
    /// Override the schema name (defaults to type name).
    pub schema_name: Option<String>,
}

impl Default for StructuredOptions<DefaultRetryHandler> {
    fn default() -> Self {
        Self {
            max_retries: 0,
            retry_handler: DefaultRetryHandler,
            schema_name: None,
        }
    }
}

impl<H: RetryHandler> StructuredOptions<H> {
    /// Set the maximum number of retries.
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Set a custom retry handler.
    pub fn with_retry_handler<H2: RetryHandler>(self, handler: H2) -> StructuredOptions<H2> {
        StructuredOptions {
            max_retries: self.max_retries,
            retry_handler: handler,
            schema_name: self.schema_name,
        }
    }

    /// Override the schema name.
    pub fn with_schema_name(mut self, name: impl Into<String>) -> Self {
        self.schema_name = Some(name.into());
        self
    }
}

/// Result of a successful structured output request.
#[derive(Debug, Clone)]
pub struct StructuredResult<T> {
    /// The parsed, validated value.
    pub value: T,
    /// Number of attempts made (1 = first attempt succeeded).
    pub attempts: u32,
    /// Request ID from the server (if available).
    pub request_id: Option<String>,
}

// ============================================================================
// Retry Orchestration
// ============================================================================

/// Decision from processing a structured output response.
pub(crate) enum RetryDecision<T> {
    /// Successfully parsed the response.
    Success(StructuredResult<T>),
    /// Should retry with these additional messages.
    Retry(Vec<InputItem>),
    /// All retries exhausted or handler stopped retrying.
    Exhausted(StructuredExhaustedError),
}

/// Manages retry state for structured output requests.
///
/// This extracts the retry orchestration logic so it can be shared between
/// async and blocking execution paths.
pub(crate) struct RetryExecutor<'a, H: RetryHandler> {
    options: &'a StructuredOptions<H>,
    attempts: Vec<AttemptRecord>,
    current_attempt: u32,
    max_attempts: u32,
    last_request_id: Option<String>,
}

impl<'a, H: RetryHandler> RetryExecutor<'a, H> {
    pub(crate) fn new(options: &'a StructuredOptions<H>) -> Self {
        Self {
            options,
            attempts: Vec::new(),
            current_attempt: 0,
            max_attempts: options.max_retries + 1,
            last_request_id: None,
        }
    }

    /// Process a response and return the retry decision.
    ///
    /// This is the core retry logic extracted for reuse between async and blocking.
    pub(crate) fn process_response<T: DeserializeOwned>(
        &mut self,
        response: &Response,
        messages: &[InputItem],
    ) -> Result<RetryDecision<T>, StructuredError> {
        self.current_attempt += 1;
        self.last_request_id = response.request_id.clone();

        // Extract raw JSON from response
        let raw_json = extract_json_content(response)?;

        // Try to parse the response
        match serde_json::from_str::<T>(&raw_json) {
            Ok(value) => Ok(RetryDecision::Success(StructuredResult {
                value,
                attempts: self.current_attempt,
                request_id: self.last_request_id.clone(),
            })),
            Err(e) => {
                let error = StructuredErrorKind::Decode {
                    message: e.to_string(),
                };

                self.attempts.push(AttemptRecord {
                    attempt: self.current_attempt,
                    raw_json: raw_json.clone(),
                    error: error.clone(),
                });

                // Check if we should retry
                if self.current_attempt >= self.max_attempts {
                    return Ok(RetryDecision::Exhausted(StructuredExhaustedError {
                        last_raw_json: raw_json,
                        all_attempts: std::mem::take(&mut self.attempts),
                        final_error: error,
                    }));
                }

                // Get retry messages from handler
                match self.options.retry_handler.on_validation_error(
                    self.current_attempt,
                    &raw_json,
                    &error,
                    messages,
                ) {
                    Some(retry_messages) => Ok(RetryDecision::Retry(retry_messages)),
                    None => {
                        // Handler chose to stop retrying
                        Ok(RetryDecision::Exhausted(StructuredExhaustedError {
                            last_raw_json: raw_json,
                            all_attempts: std::mem::take(&mut self.attempts),
                            final_error: error,
                        }))
                    }
                }
            }
        }
    }
}

// ============================================================================
// Schema Generation
// ============================================================================

/// Generates an [`OutputFormat`] with automatic JSON schema from a type.
///
/// This function uses [`schemars`] to generate a JSON schema from a type that
/// implements [`JsonSchema`], then wraps it in an [`OutputFormat`] with
/// `type = "json_schema"` and `strict = true`.
///
/// # Example
///
/// ```ignore
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
/// use modelrelay::output_format_from_type;
///
/// #[derive(JsonSchema, Deserialize)]
/// struct WeatherResponse {
///     temperature: f64,
///     conditions: String,
/// }
///
/// let format = output_format_from_type::<WeatherResponse>(None)?;
/// ```
pub fn output_format_from_type<T: JsonSchema>(
    schema_name: Option<&str>,
) -> Result<OutputFormat, StructuredError> {
    let root_schema = schemars::schema_for!(T);
    let schema_value = serde_json::to_value(&root_schema)
        .map_err(|e| StructuredError::Sdk(crate::Error::Serialization(e)))?;

    // Extract the type name for the schema name
    let name = schema_name.map(|s| s.to_string()).unwrap_or_else(|| {
        std::any::type_name::<T>()
            .split("::")
            .last()
            .unwrap_or("response")
            .to_string()
    });

    Ok(OutputFormat {
        kind: OutputFormatKind::JsonSchema,
        json_schema: Some(JSONSchemaFormat {
            name,
            description: None,
            schema: schema_value,
            strict: Some(true),
        }),
    })
}

// ============================================================================
// Builder (Async)
// ============================================================================

/// Builder for structured output using the Responses API.
pub struct StructuredResponseBuilder<T, H: RetryHandler = DefaultRetryHandler> {
    pub(crate) inner: crate::responses::ResponseBuilder,
    pub(crate) options: StructuredOptions<H>,
    pub(crate) _marker: PhantomData<T>,
}

impl<T: JsonSchema + DeserializeOwned> StructuredResponseBuilder<T, DefaultRetryHandler> {
    pub fn new(inner: crate::responses::ResponseBuilder) -> Self {
        Self {
            inner,
            options: StructuredOptions::default(),
            _marker: PhantomData,
        }
    }
}

impl<T: JsonSchema + DeserializeOwned, H: RetryHandler> StructuredResponseBuilder<T, H> {
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.options.max_retries = retries;
        self
    }

    pub fn retry_handler<H2: RetryHandler>(self, handler: H2) -> StructuredResponseBuilder<T, H2> {
        StructuredResponseBuilder {
            inner: self.inner,
            options: self.options.with_retry_handler(handler),
            _marker: PhantomData,
        }
    }

    pub fn schema_name(mut self, name: impl Into<String>) -> Self {
        self.options.schema_name = Some(name.into());
        self
    }

    // Forward core builder methods (convenience)

    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.inner = self.inner.provider(provider);
        self
    }

    pub fn model(mut self, model: impl Into<crate::types::Model>) -> Self {
        self.inner = self.inner.model(model);
        self
    }

    pub fn customer_id(mut self, customer_id: impl Into<String>) -> Self {
        self.inner = self.inner.customer_id(customer_id);
        self
    }

    pub fn system(mut self, content: impl Into<String>) -> Self {
        self.inner = self.inner.system(content);
        self
    }

    pub fn user(mut self, content: impl Into<String>) -> Self {
        self.inner = self.inner.user(content);
        self
    }

    pub fn assistant(mut self, content: impl Into<String>) -> Self {
        self.inner = self.inner.assistant(content);
        self
    }

    pub fn tool_result(
        mut self,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.inner = self.inner.tool_result(tool_call_id, content);
        self
    }

    pub fn item(mut self, item: InputItem) -> Self {
        self.inner = self.inner.item(item);
        self
    }

    pub fn input(mut self, input: Vec<InputItem>) -> Self {
        self.inner = self.inner.input(input);
        self
    }

    pub fn max_output_tokens(mut self, max_output_tokens: i64) -> Self {
        self.inner = self.inner.max_output_tokens(max_output_tokens);
        self
    }

    pub fn temperature(mut self, temperature: f64) -> Self {
        self.inner = self.inner.temperature(temperature);
        self
    }

    pub fn stop(mut self, stop: Vec<String>) -> Self {
        self.inner = self.inner.stop(stop);
        self
    }

    pub fn tools(mut self, tools: Vec<crate::types::Tool>) -> Self {
        self.inner = self.inner.tools(tools);
        self
    }

    pub fn tool_choice(mut self, tool_choice: crate::types::ToolChoice) -> Self {
        self.inner = self.inner.tool_choice(tool_choice);
        self
    }

    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.inner = self.inner.request_id(request_id);
        self
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner = self.inner.header(key, value);
        self
    }

    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.inner = self.inner.timeout(timeout);
        self
    }

    pub fn retry(mut self, retry: crate::RetryConfig) -> Self {
        self.inner = self.inner.retry(retry);
        self
    }

    pub async fn send(
        self,
        client: &crate::client::ResponsesClient,
    ) -> Result<StructuredResult<T>, StructuredError> {
        client
            .create_structured::<T, H>(self.inner, self.options)
            .await
    }

    #[cfg(feature = "blocking")]
    pub fn send_blocking(
        self,
        client: &BlockingResponsesClient,
    ) -> Result<StructuredResult<T>, StructuredError> {
        client.create_structured::<T, H>(self.inner, self.options)
    }

    #[cfg(feature = "streaming")]
    pub async fn stream(
        self,
        client: &crate::client::ResponsesClient,
    ) -> Result<crate::responses::StructuredJSONStream<T>, StructuredError> {
        let output_format = output_format_from_type::<T>(self.options.schema_name.as_deref())?;
        self.inner
            .output_format(output_format)
            .stream_json(client)
            .await
            .map_err(StructuredError::Sdk)
    }

    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_blocking(
        self,
        client: &BlockingResponsesClient,
    ) -> Result<BlockingStructuredJSONStream<T>, StructuredError> {
        let output_format = output_format_from_type::<T>(self.options.schema_name.as_deref())?;
        self.inner
            .output_format(output_format)
            .stream_json_blocking(client)
            .map_err(StructuredError::Sdk)
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Extract JSON content from a proxy response.
///
/// Returns an error if the response content is empty, providing a clear
/// error message rather than a confusing JSON parse error.
fn extract_json_content(response: &Response) -> Result<String, StructuredError> {
    let mut content = String::new();
    for item in &response.output {
        let OutputItem::Message {
            role,
            content: parts,
            ..
        } = item;
        if *role != MessageRole::Assistant {
            continue;
        }
        for part in parts {
            match part {
                ContentPart::Text { text } => content.push_str(text),
            }
        }
    }
    if content.trim().is_empty() {
        return Err(StructuredError::Sdk(crate::Error::Transport(
            crate::errors::TransportError {
                kind: crate::errors::TransportErrorKind::EmptyResponse,
                message: "response contained no content".to_string(),
                source: None,
                retries: None,
            },
        )));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
    struct TestPerson {
        name: String,
        age: u32,
    }

    #[test]
    fn test_output_format_from_type() {
        let format = output_format_from_type::<TestPerson>(None).unwrap();
        assert_eq!(format.kind, OutputFormatKind::JsonSchema);
        assert!(format.json_schema.is_some());
        let schema = format.json_schema.unwrap();
        assert_eq!(schema.name, "TestPerson");
        assert_eq!(schema.strict, Some(true));
    }

    #[test]
    fn test_output_format_custom_name() {
        let format = output_format_from_type::<TestPerson>(Some("person_info")).unwrap();
        let schema = format.json_schema.unwrap();
        assert_eq!(schema.name, "person_info");
    }

    #[test]
    fn test_structured_error_kind_display() {
        let decode_error = StructuredErrorKind::Decode {
            message: "expected string".to_string(),
        };
        assert!(decode_error.to_string().contains("decode error"));

        let validation_error = StructuredErrorKind::Validation {
            issues: vec![ValidationIssue {
                path: Some("person.age".to_string()),
                message: "expected integer".to_string(),
            }],
        };
        assert!(validation_error.to_string().contains("person.age"));
    }

    #[test]
    fn test_default_retry_handler() {
        let handler = DefaultRetryHandler;
        let error = StructuredErrorKind::Decode {
            message: "parse error".to_string(),
        };
        let messages = handler.on_validation_error(1, "{}", &error, &[]);
        assert!(messages.is_some());
        let msgs = messages.unwrap();
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            InputItem::Message { role, content, .. } => {
                assert_eq!(*role, MessageRole::User);
                let text = content
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text { text } => Some(text.as_str()),
                    })
                    .collect::<String>();
                assert!(text.contains("schema"));
            }
        }
    }
}
