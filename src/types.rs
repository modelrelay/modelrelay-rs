use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::errors::{Error, ValidationError};

/// Stop reason returned by the backend and surfaced by `/llm/proxy`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum StopReason {
    Completed,
    Stop,
    StopSequence,
    EndTurn,
    MaxTokens,
    MaxLength,
    MaxContext,
    ToolCalls,
    TimeLimit,
    ContentFilter,
    Incomplete,
    Unknown,
    Other(String),
}

impl StopReason {
    pub fn as_str(&self) -> &str {
        match self {
            StopReason::Completed => "completed",
            StopReason::Stop => "stop",
            StopReason::StopSequence => "stop_sequence",
            StopReason::EndTurn => "end_turn",
            StopReason::MaxTokens => "max_tokens",
            StopReason::MaxLength => "max_len",
            StopReason::MaxContext => "max_context",
            StopReason::ToolCalls => "tool_calls",
            StopReason::TimeLimit => "time_limit",
            StopReason::ContentFilter => "content_filter",
            StopReason::Incomplete => "incomplete",
            StopReason::Unknown => "unknown",
            StopReason::Other(other) => other.as_str(),
        }
    }
}

impl From<&str> for StopReason {
    fn from(value: &str) -> Self {
        StopReason::from(value.to_string())
    }
}

impl From<String> for StopReason {
    fn from(value: String) -> Self {
        let normalized = value.trim().to_lowercase();
        match normalized.as_str() {
            "completed" => StopReason::Completed,
            "stop" => StopReason::Stop,
            "stop_sequence" => StopReason::StopSequence,
            "end_turn" => StopReason::EndTurn,
            "max_tokens" => StopReason::MaxTokens,
            "max_len" | "length" => StopReason::MaxLength,
            "max_context" => StopReason::MaxContext,
            "tool_calls" => StopReason::ToolCalls,
            "time_limit" => StopReason::TimeLimit,
            "content_filter" => StopReason::ContentFilter,
            "incomplete" => StopReason::Incomplete,
            "unknown" => StopReason::Unknown,
            other => StopReason::Other(other.to_string()),
        }
    }
}

impl From<StopReason> for String {
    fn from(value: StopReason) -> Self {
        value.as_str().to_string()
    }
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Message role in a chat conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl MessageRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
        }
    }
}

impl fmt::Display for MessageRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Model identifier (string wrapper).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct Model(String);

impl Model {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl From<&str> for Model {
    fn from(value: &str) -> Self {
        Model::from(value.to_string())
    }
}

impl From<String> for Model {
    fn from(value: String) -> Self {
        Model(value.trim().to_string())
    }
}

impl From<Model> for String {
    fn from(value: Model) -> Self {
        value.0
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A single chat turn used by the LLM proxy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Response format configuration for structured outputs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: ResponseFormatKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<ResponseJSONSchema>,
}

impl ResponseFormat {
    /// Create a text response format (default, non-structured).
    pub fn text() -> Self {
        Self {
            kind: ResponseFormatKind::Text,
            json_schema: None,
        }
    }

    /// Create a JSON schema response format for structured outputs.
    ///
    /// # Arguments
    /// * `name` - Schema name (typically the type name)
    /// * `schema` - JSON schema object
    pub fn json_schema(name: impl Into<String>, schema: Value) -> Self {
        Self {
            kind: ResponseFormatKind::JsonSchema,
            json_schema: Some(ResponseJSONSchema {
                name: name.into(),
                description: None,
                schema,
                strict: Some(true),
            }),
        }
    }

    /// Create a JSON schema response format with description.
    pub fn json_schema_with_description(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: Value,
    ) -> Self {
        Self {
            kind: ResponseFormatKind::JsonSchema,
            json_schema: Some(ResponseJSONSchema {
                name: name.into(),
                description: Some(description.into()),
                schema,
                strict: Some(true),
            }),
        }
    }

    /// Returns true if this is a structured (JSON schema) response format.
    pub fn is_structured(&self) -> bool {
        self.kind == ResponseFormatKind::JsonSchema
    }
}

/// Supported response format types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ResponseFormatKind {
    Text,
    JsonSchema,
    Other(String),
}

impl ResponseFormatKind {
    pub fn as_str(&self) -> &str {
        match self {
            ResponseFormatKind::Text => "text",
            ResponseFormatKind::JsonSchema => "json_schema",
            ResponseFormatKind::Other(other) => other.as_str(),
        }
    }
}

impl From<&str> for ResponseFormatKind {
    fn from(value: &str) -> Self {
        ResponseFormatKind::from(value.to_string())
    }
}

impl From<String> for ResponseFormatKind {
    fn from(value: String) -> Self {
        let trimmed = value.trim();
        match trimmed.to_lowercase().as_str() {
            "text" => ResponseFormatKind::Text,
            "json_schema" => ResponseFormatKind::JsonSchema,
            _ => ResponseFormatKind::Other(trimmed.to_string()),
        }
    }
}

impl From<ResponseFormatKind> for String {
    fn from(value: ResponseFormatKind) -> Self {
        value.as_str().to_string()
    }
}

impl fmt::Display for ResponseFormatKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// JSON schema payload for structured outputs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResponseJSONSchema {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

// --- Tool Types ---

/// Tool type identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolType {
    Function,
    Web,
    XSearch,
    CodeExecution,
}

impl ToolType {
    pub fn as_str(&self) -> &str {
        match self {
            ToolType::Function => "function",
            ToolType::Web => "web",
            ToolType::XSearch => "x_search",
            ToolType::CodeExecution => "code_execution",
        }
    }
}

impl fmt::Display for ToolType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Function tool definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
}

/// Web tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WebToolConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// X/Twitter search configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct XSearchConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_handles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_handles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_date: Option<String>,
}

/// Code execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeExecConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<i64>,
}

/// A tool available for the model to call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: ToolType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<WebToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_search: Option<XSearchConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_execution: Option<CodeExecConfig>,
}

impl Tool {
    /// Create a function tool.
    pub fn function(
        name: impl Into<String>,
        description: Option<String>,
        parameters: Option<Value>,
    ) -> Self {
        Self {
            kind: ToolType::Function,
            function: Some(FunctionTool {
                name: name.into(),
                description,
                parameters,
            }),
            web: None,
            x_search: None,
            code_execution: None,
        }
    }

    /// Create a web tool.
    pub fn web(config: WebToolConfig) -> Self {
        Self {
            kind: ToolType::Web,
            function: None,
            web: Some(config),
            x_search: None,
            code_execution: None,
        }
    }

    /// Create an X/Twitter search tool.
    pub fn x_search(config: XSearchConfig) -> Self {
        Self {
            kind: ToolType::XSearch,
            function: None,
            web: None,
            x_search: Some(config),
            code_execution: None,
        }
    }
}

/// Tool choice type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoiceType {
    Auto,
    Required,
    None,
}

impl ToolChoiceType {
    pub fn as_str(&self) -> &str {
        match self {
            ToolChoiceType::Auto => "auto",
            ToolChoiceType::Required => "required",
            ToolChoiceType::None => "none",
        }
    }
}

impl fmt::Display for ToolChoiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Controls how the model responds to tool calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolChoice {
    #[serde(rename = "type")]
    pub kind: ToolChoiceType,
}

impl ToolChoice {
    pub fn auto() -> Self {
        Self {
            kind: ToolChoiceType::Auto,
        }
    }

    pub fn required() -> Self {
        Self {
            kind: ToolChoiceType::Required,
        }
    }

    pub fn none() -> Self {
        Self {
            kind: ToolChoiceType::None,
        }
    }
}

/// Function call details in a tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// A tool call made by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ToolType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionCall>,
}

/// Request payload for `/llm/proxy`.
/// This is internal - users should use `ChatRequestBuilder` instead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ProxyRequest {
    pub(crate) model: Model,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f64>,
    pub(crate) messages: Vec<ProxyMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<ToolChoice>,
}

impl ProxyRequest {
    pub fn validate(&self) -> Result<(), Error> {
        if self.model.is_empty() {
            return Err(Error::Validation(
                ValidationError::new("model is required").with_field("model"),
            ));
        }
        if self.messages.is_empty() {
            return Err(Error::Validation(
                ValidationError::new("at least one message is required").with_field("messages"),
            ));
        }
        if let Some(format) = &self.response_format {
            validate_response_format(format)?;
        }
        Ok(())
    }
}

fn validate_response_format(format: &ResponseFormat) -> Result<(), Error> {
    match &format.kind {
        ResponseFormatKind::Text => Ok(()),
        ResponseFormatKind::JsonSchema => {
            let Some(schema) = &format.json_schema else {
                return Err(Error::Validation(
                    ValidationError::new(
                        "response_format.json_schema required when type=json_schema",
                    )
                    .with_field("response_format.json_schema"),
                ));
            };

            if schema.name.trim().is_empty() {
                return Err(Error::Validation(
                    ValidationError::new("response_format.json_schema.name required")
                        .with_field("response_format.json_schema.name"),
                ));
            }
            if schema.schema.is_null() {
                return Err(Error::Validation(
                    ValidationError::new("response_format.json_schema.schema required")
                        .with_field("response_format.json_schema.schema"),
                ));
            }
            if !schema.schema.is_object() {
                return Err(Error::Validation(
                    ValidationError::new("response_format.json_schema.schema must be an object")
                        .with_field("response_format.json_schema.schema"),
                ));
            }
            Ok(())
        }
        ResponseFormatKind::Other(other) => Err(Error::Validation(
            ValidationError::new(format!("invalid response_format.type: {}", other))
                .with_field("response_format.type"),
        )),
    }
}

/// Aggregated response returned by `/llm/proxy`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyResponse {
    pub id: String,
    pub content: Vec<String>,
    #[serde(
        default,
        rename = "stop_reason",
        alias = "stopReason",
        skip_serializing_if = "Option::is_none"
    )]
    pub stop_reason: Option<StopReason>,
    pub model: Model,
    pub usage: Usage,
    /// Request identifier echoed by the API (response header).
    #[serde(default, skip_serializing)]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// Token usage metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Usage {
    #[serde(default, rename = "input_tokens", alias = "inputTokens")]
    pub input_tokens: i64,
    #[serde(default, rename = "output_tokens", alias = "outputTokens")]
    pub output_tokens: i64,
    #[serde(default, rename = "total_tokens", alias = "totalTokens")]
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsageSummary {
    pub plan: String,
    #[serde(
        default,
        rename = "plan_type",
        alias = "planType",
        skip_serializing_if = "Option::is_none"
    )]
    pub plan_type: Option<String>,
    #[serde(
        default,
        rename = "window_start",
        alias = "windowStart",
        with = "time::serde::rfc3339::option"
    )]
    pub window_start: Option<OffsetDateTime>,
    #[serde(
        default,
        rename = "window_end",
        alias = "windowEnd",
        with = "time::serde::rfc3339::option"
    )]
    pub window_end: Option<OffsetDateTime>,
    #[serde(default)]
    pub limit: i64,
    #[serde(default)]
    pub used: i64,
    #[serde(
        default,
        rename = "actions_limit",
        alias = "actionsLimit",
        skip_serializing_if = "Option::is_none"
    )]
    pub actions_limit: Option<i64>,
    #[serde(
        default,
        rename = "actions_used",
        alias = "actionsUsed",
        skip_serializing_if = "Option::is_none"
    )]
    pub actions_used: Option<i64>,
    #[serde(default)]
    pub remaining: i64,
    #[serde(default)]
    pub state: String,
}

impl Usage {
    /// Prompt tokens counted by the backend.
    pub fn input(&self) -> i64 {
        self.input_tokens
    }

    /// Completion tokens counted by the backend.
    pub fn output(&self) -> i64 {
        self.output_tokens
    }

    /// Total tokens (computed from input/output if the field was omitted).
    pub fn total(&self) -> i64 {
        if self.total_tokens > 0 {
            self.total_tokens
        } else {
            self.input_tokens.saturating_add(self.output_tokens)
        }
    }
}

/// High-level streaming event kinds emitted by the API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamEventKind {
    MessageStart,
    MessageDelta,
    MessageStop,
    ToolUseStart,
    ToolUseDelta,
    ToolUseStop,
    Ping,
    Custom,
}

impl StreamEventKind {
    pub fn from_event_name(name: &str) -> Self {
        match name {
            // Unified NDJSON format record types
            "start" => Self::MessageStart,
            "update" => Self::MessageDelta,
            "completion" => Self::MessageStop,
            // Legacy event names (for backwards compatibility during transition)
            "message_start" => Self::MessageStart,
            "message_delta" => Self::MessageDelta,
            "message_stop" => Self::MessageStop,
            "tool_use_start" => Self::ToolUseStart,
            "tool_use_delta" => Self::ToolUseDelta,
            "tool_use_stop" => Self::ToolUseStop,
            "ping" => Self::Ping,
            "custom" => Self::Custom,
            _ => Self::Custom,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            StreamEventKind::MessageStart => "message_start",
            StreamEventKind::MessageDelta => "message_delta",
            StreamEventKind::MessageStop => "message_stop",
            StreamEventKind::ToolUseStart => "tool_use_start",
            StreamEventKind::ToolUseDelta => "tool_use_delta",
            StreamEventKind::ToolUseStop => "tool_use_stop",
            StreamEventKind::Ping => "ping",
            StreamEventKind::Custom => "custom",
        }
    }
}

/// Incremental update to a tool call during streaming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallDelta {
    pub index: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionCallDelta>,
}

/// Incremental function call data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCallDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Single SSE event emitted by the streaming proxy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub kind: StreamEventKind,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_delta: Option<String>,
    /// Incremental tool call update during streaming.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_delta: Option<ToolCallDelta>,
    /// Completed tool calls when kind is ToolUseStop or MessageStop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Unparsed SSE payload string.
    pub raw: String,
}

impl StreamEvent {
    pub fn event_name(&self) -> &str {
        if self.event.is_empty() {
            self.kind.as_str()
        } else {
            &self.event
        }
    }
}

/// Representation of an API key record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct APIKey {
    pub id: Uuid,
    pub label: String,
    pub kind: String,
    #[serde(rename = "created_at", with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(
        default,
        rename = "expires_at",
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(
        default,
        rename = "last_used_at",
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_used_at: Option<OffsetDateTime>,
    #[serde(rename = "redacted_key")]
    pub redacted_key: String,
    #[serde(
        default,
        rename = "secret_key",
        skip_serializing_if = "Option::is_none"
    )]
    pub secret_key: Option<String>,
}

/// Request payload for POST /auth/frontend-token for an existing customer.
///
/// Use [`FrontendTokenRequest::new`] to create a request with required fields.
/// For auto-provisioning new customers, use [`FrontendTokenAutoProvisionRequest`] instead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrontendTokenRequest {
    /// Publishable key (mr_pk_*) - required for authentication.
    pub publishable_key: String,
    /// Customer identifier - required to issue a token for this customer.
    #[serde(rename = "customer_id")]
    pub customer_id: String,
    /// Optional device identifier for tracking/rate limiting.
    #[serde(skip_serializing_if = "Option::is_none", rename = "device_id")]
    pub device_id: Option<String>,
    /// Optional TTL in seconds for the issued token.
    #[serde(skip_serializing_if = "Option::is_none", rename = "ttl_seconds")]
    pub ttl_seconds: Option<i64>,
}

impl FrontendTokenRequest {
    /// Create a new frontend token request with required fields.
    pub fn new(publishable_key: impl Into<String>, customer_id: impl Into<String>) -> Self {
        Self {
            publishable_key: publishable_key.into(),
            customer_id: customer_id.into(),
            device_id: None,
            ttl_seconds: None,
        }
    }

    /// Set the device ID for tracking/rate limiting.
    pub fn with_device_id(mut self, device_id: impl Into<String>) -> Self {
        self.device_id = Some(device_id.into());
        self
    }

    /// Set the TTL in seconds for the issued token.
    pub fn with_ttl_seconds(mut self, ttl: i64) -> Self {
        self.ttl_seconds = Some(ttl);
        self
    }

    /// Convert to an auto-provisioning request by adding an email.
    /// Use this when the customer may not exist and should be created on the free tier.
    pub fn with_auto_provision(
        self,
        email: impl Into<String>,
    ) -> FrontendTokenAutoProvisionRequest {
        FrontendTokenAutoProvisionRequest {
            publishable_key: self.publishable_key,
            customer_id: self.customer_id,
            email: email.into(),
            device_id: self.device_id,
            ttl_seconds: self.ttl_seconds,
        }
    }
}

/// Request payload for POST /auth/frontend-token with auto-provisioning.
///
/// Use this when the customer may not exist and should be created on the free tier.
/// The email is required for auto-provisioning.
///
/// Create via [`FrontendTokenAutoProvisionRequest::new`] or
/// [`FrontendTokenRequest::with_auto_provision`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrontendTokenAutoProvisionRequest {
    /// Publishable key (mr_pk_*) - required for authentication.
    pub publishable_key: String,
    /// Customer identifier - required to issue a token for this customer.
    #[serde(rename = "customer_id")]
    pub customer_id: String,
    /// Email address - required for auto-provisioning a new customer.
    pub email: String,
    /// Optional device identifier for tracking/rate limiting.
    #[serde(skip_serializing_if = "Option::is_none", rename = "device_id")]
    pub device_id: Option<String>,
    /// Optional TTL in seconds for the issued token.
    #[serde(skip_serializing_if = "Option::is_none", rename = "ttl_seconds")]
    pub ttl_seconds: Option<i64>,
}

impl FrontendTokenAutoProvisionRequest {
    /// Create a new auto-provisioning frontend token request with required fields.
    pub fn new(
        publishable_key: impl Into<String>,
        customer_id: impl Into<String>,
        email: impl Into<String>,
    ) -> Self {
        Self {
            publishable_key: publishable_key.into(),
            customer_id: customer_id.into(),
            email: email.into(),
            device_id: None,
            ttl_seconds: None,
        }
    }

    /// Set the device ID for tracking/rate limiting.
    pub fn with_device_id(mut self, device_id: impl Into<String>) -> Self {
        self.device_id = Some(device_id.into());
        self
    }

    /// Set the TTL in seconds for the issued token.
    pub fn with_ttl_seconds(mut self, ttl: i64) -> Self {
        self.ttl_seconds = Some(ttl);
        self
    }
}

/// OAuth2 token type. Always "Bearer" for ModelRelay tokens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TokenType {
    #[serde(rename = "Bearer")]
    Bearer,
}

/// Short-lived bearer token usable from browser/mobile clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrontendToken {
    /// The bearer token for authenticating LLM requests.
    pub token: String,
    /// When the token expires.
    #[serde(
        rename = "expires_at",
        alias = "expiresAt",
        with = "time::serde::rfc3339"
    )]
    pub expires_at: OffsetDateTime,
    /// Seconds until the token expires (also computable from expires_at).
    #[serde(rename = "expires_in", alias = "expiresIn")]
    pub expires_in: u32,
    /// Token type, always Bearer.
    #[serde(rename = "token_type", alias = "tokenType")]
    pub token_type: TokenType,
    /// The publishable key ID that issued this token.
    #[serde(rename = "key_id", alias = "keyId")]
    pub key_id: Uuid,
    /// Unique session identifier for this token.
    #[serde(rename = "session_id", alias = "sessionId")]
    pub session_id: Uuid,
    /// The project ID this token is scoped to.
    #[serde(rename = "project_id", alias = "projectId")]
    pub project_id: Uuid,
    /// The internal customer ID (UUID).
    #[serde(rename = "customer_id", alias = "customerId")]
    pub customer_id: Uuid,
    /// The external customer ID provided by the application.
    #[serde(rename = "customer_external_id", alias = "customerExternalId")]
    pub customer_external_id: String,
    /// The tier code for the customer (e.g., "free", "pro", "enterprise").
    #[serde(rename = "tier_code", alias = "tierCode")]
    pub tier_code: String,
    /// Device identifier used when issuing the token. Added client-side for caching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Publishable key used for issuance. Added client-side for caching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publishable_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_round_trips_and_allows_other() {
        let reason: StopReason = serde_json::from_str("\"end_turn\"").unwrap();
        assert_eq!(reason, StopReason::EndTurn);
        let filtered: StopReason = serde_json::from_str("\"content_filter\"").unwrap();
        assert_eq!(filtered, StopReason::ContentFilter);
        let other: StopReason = serde_json::from_str("\"vendor_reason\"").unwrap();
        assert!(matches!(other, StopReason::Other(val) if val == "vendor_reason"));
        let serialized = serde_json::to_string(&StopReason::MaxLength).unwrap();
        assert_eq!(serialized, "\"max_len\"");
    }

    #[test]
    fn model_round_trip_and_trim() {
        let model: Model = serde_json::from_str("\" gpt-4o-mini \"").unwrap();
        assert_eq!(model.as_str(), "gpt-4o-mini");
        let other_model: Model = serde_json::from_str("\"my/model\"").unwrap();
        assert_eq!(other_model.as_str(), "my/model");
    }

    #[test]
    fn proxy_request_validation_guards_required_fields() {
        // Empty model fails validation
        let req = ProxyRequest {
            model: Model::from(""),
            messages: vec![ProxyMessage {
                role: MessageRole::User,
                content: "hi".into(),
                tool_calls: None,
                tool_call_id: None,
            }],
            max_tokens: None,
            temperature: None,
            response_format: None,
            stop: None,
            tools: None,
            tool_choice: None,
        };
        assert!(req.validate().is_err());

        // Empty messages fails validation
        let req = ProxyRequest {
            model: Model::from("gpt-4o-mini"),
            messages: Vec::new(),
            max_tokens: None,
            temperature: None,
            response_format: None,
            stop: None,
            tools: None,
            tool_choice: None,
        };
        assert!(req.validate().is_err());

        // Valid request passes validation
        let req = ProxyRequest {
            model: Model::from("gpt-4o-mini"),
            messages: vec![ProxyMessage {
                role: MessageRole::User,
                content: "hi".into(),
                tool_calls: None,
                tool_call_id: None,
            }],
            max_tokens: None,
            temperature: None,
            response_format: None,
            stop: None,
            tools: None,
            tool_choice: None,
        };
        assert!(req.validate().is_ok());
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(
            json.get("model").and_then(|v| v.as_str()),
            Some("gpt-4o-mini")
        );
    }
}
