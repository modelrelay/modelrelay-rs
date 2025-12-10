//! Ergonomic structured output API with schema inference and validation.
//!
//! This module provides type-safe structured outputs using [`schemars`] for automatic
//! JSON schema generation. The API handles schema construction, validation retries
//! with error feedback, and strongly-typed result parsing.
//!
//! # Example
//!
//! ```ignore
//! use schemars::JsonSchema;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize, JsonSchema)]
//! struct Person {
//!     name: String,
//!     age: u32,
//! }
//!
//! let result = client.chat()
//!     .model("claude-sonnet-4-20250514")
//!     .user("Extract: John Doe, 30 years old")
//!     .structured::<Person>()
//!     .max_retries(2)
//!     .send()
//!     .await?;
//!
//! println!("Name: {}, Age: {}", result.value.name, result.value.age);
//! ```

use std::marker::PhantomData;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::types::{
    MessageRole, ProxyMessage, ProxyResponse, ResponseFormat, ResponseFormatKind,
    ResponseJSONSchema,
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
        messages: &[ProxyMessage],
    ) -> Option<Vec<ProxyMessage>>;
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
        _messages: &[ProxyMessage],
    ) -> Option<Vec<ProxyMessage>> {
        let error_msg = format!(
            "The previous response did not match the expected schema. Error: {}. \
             Please provide a response that matches the schema exactly.",
            error
        );
        Some(vec![ProxyMessage {
            role: MessageRole::User,
            content: error_msg,
            tool_calls: None,
            tool_call_id: None,
        }])
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
// Schema Generation
// ============================================================================

/// Generates a [`ResponseFormat`] with automatic JSON schema from a type.
///
/// This function uses [`schemars`] to generate a JSON schema from a type that
/// implements [`JsonSchema`], then wraps it in a [`ResponseFormat`] with
/// `type = "json_schema"` and `strict = true`.
///
/// # Example
///
/// ```ignore
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
/// use modelrelay::response_format_from_type;
///
/// #[derive(JsonSchema, Deserialize)]
/// struct WeatherResponse {
///     temperature: f64,
///     conditions: String,
/// }
///
/// let format = response_format_from_type::<WeatherResponse>(None)?;
/// ```
pub fn response_format_from_type<T: JsonSchema>(
    schema_name: Option<&str>,
) -> Result<ResponseFormat, StructuredError> {
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

    Ok(ResponseFormat {
        kind: ResponseFormatKind::JsonSchema,
        json_schema: Some(ResponseJSONSchema {
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

/// Builder for structured output chat requests.
///
/// Created via [`ChatRequestBuilder::structured()`].
#[cfg(feature = "client")]
pub struct StructuredChatBuilder<T, H: RetryHandler = DefaultRetryHandler> {
    pub(crate) inner: crate::chat::ChatRequestBuilder,
    pub(crate) options: StructuredOptions<H>,
    pub(crate) _marker: PhantomData<T>,
}

#[cfg(feature = "client")]
impl<T: JsonSchema + DeserializeOwned> StructuredChatBuilder<T, DefaultRetryHandler> {
    /// Create a new structured builder from a chat request builder.
    pub fn new(inner: crate::chat::ChatRequestBuilder) -> Self {
        Self {
            inner,
            options: StructuredOptions::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "client")]
impl<T: JsonSchema + DeserializeOwned, H: RetryHandler> StructuredChatBuilder<T, H> {
    /// Set the maximum number of retries on validation failure.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.options.max_retries = retries;
        self
    }

    /// Set a custom retry handler.
    pub fn retry_handler<H2: RetryHandler>(self, handler: H2) -> StructuredChatBuilder<T, H2> {
        StructuredChatBuilder {
            inner: self.inner,
            options: self.options.with_retry_handler(handler),
            _marker: PhantomData,
        }
    }

    /// Override the schema name.
    pub fn schema_name(mut self, name: impl Into<String>) -> Self {
        self.options.schema_name = Some(name.into());
        self
    }

    // Forward builder methods from ChatRequestBuilder

    /// Add a system message.
    pub fn system(mut self, content: impl Into<String>) -> Self {
        self.inner = self.inner.system(content);
        self
    }

    /// Add a user message.
    pub fn user(mut self, content: impl Into<String>) -> Self {
        self.inner = self.inner.user(content);
        self
    }

    /// Add an assistant message.
    pub fn assistant(mut self, content: impl Into<String>) -> Self {
        self.inner = self.inner.assistant(content);
        self
    }

    /// Set max tokens.
    pub fn max_tokens(mut self, max_tokens: i64) -> Self {
        self.inner = self.inner.max_tokens(max_tokens);
        self
    }

    /// Set temperature.
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.inner = self.inner.temperature(temperature);
        self
    }

    /// Set request timeout.
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.inner = self.inner.timeout(timeout);
        self
    }

    /// Execute the structured request (blocking, async).
    ///
    /// Returns a typed result with validation retries if configured.
    pub async fn send(
        self,
        client: &crate::client::LLMClient,
    ) -> Result<StructuredResult<T>, StructuredError> {
        // Set the response format with schema
        let response_format = response_format_from_type::<T>(self.options.schema_name.as_deref())?;
        let mut inner = self.inner.response_format(response_format);

        let mut attempts: Vec<AttemptRecord> = Vec::new();
        let mut current_attempt = 0u32;
        let max_attempts = self.options.max_retries + 1;

        loop {
            current_attempt += 1;

            // Build and send request
            let response: ProxyResponse = inner.clone().send(client).await?;
            let request_id = response.request_id.clone();

            // Extract raw JSON from response
            let raw_json = extract_json_content(&response);

            // Try to parse the response
            match serde_json::from_str::<T>(&raw_json) {
                Ok(value) => {
                    return Ok(StructuredResult {
                        value,
                        attempts: current_attempt,
                        request_id,
                    });
                }
                Err(e) => {
                    let error = StructuredErrorKind::Decode {
                        message: e.to_string(),
                    };

                    attempts.push(AttemptRecord {
                        attempt: current_attempt,
                        raw_json: raw_json.clone(),
                        error: error.clone(),
                    });

                    // Check if we should retry
                    if current_attempt >= max_attempts {
                        return Err(StructuredError::Exhausted(StructuredExhaustedError {
                            last_raw_json: raw_json,
                            all_attempts: attempts,
                            final_error: error,
                        }));
                    }

                    // Get retry messages from handler
                    let messages = inner.messages.clone();
                    match self.options.retry_handler.on_validation_error(
                        current_attempt,
                        &raw_json,
                        &error,
                        &messages,
                    ) {
                        Some(retry_messages) => {
                            // Append retry messages and continue
                            for msg in retry_messages {
                                inner = inner.message(msg.role.clone(), msg.content.clone());
                            }
                        }
                        None => {
                            // Handler chose to stop retrying
                            return Err(StructuredError::Exhausted(StructuredExhaustedError {
                                last_raw_json: raw_json,
                                all_attempts: attempts,
                                final_error: error,
                            }));
                        }
                    }
                }
            }
        }
    }

    /// Execute and stream structured responses.
    ///
    /// Note: Streaming does not support retries. For retry behavior, use [`send`].
    #[cfg(feature = "streaming")]
    pub async fn stream(
        self,
        client: &crate::client::LLMClient,
    ) -> Result<crate::chat::StructuredJSONStream<T>, StructuredError> {
        let response_format = response_format_from_type::<T>(self.options.schema_name.as_deref())?;
        self.inner
            .response_format(response_format)
            .stream_json(client)
            .await
            .map_err(StructuredError::Sdk)
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Extract JSON content from a proxy response.
fn extract_json_content(response: &ProxyResponse) -> String {
    // Content is Vec<String>, join all parts
    response.content.join("")
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
    fn test_response_format_from_type() {
        let format = response_format_from_type::<TestPerson>(None).unwrap();
        assert_eq!(format.kind, ResponseFormatKind::JsonSchema);
        assert!(format.json_schema.is_some());
        let schema = format.json_schema.unwrap();
        assert_eq!(schema.name, "TestPerson");
        assert_eq!(schema.strict, Some(true));
    }

    #[test]
    fn test_response_format_custom_name() {
        let format = response_format_from_type::<TestPerson>(Some("person_info")).unwrap();
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
        assert_eq!(msgs[0].role, MessageRole::User);
        assert!(msgs[0].content.contains("schema"));
    }
}
