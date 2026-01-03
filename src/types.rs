//! SDK types for the ModelRelay API.
//!
//! ## Type Strategy
//!
//! This module follows a hybrid approach for API types:
//!
//! - **Generated types** (from `crate::generated`): Used where OpenAPI-generated types
//!   match SDK ergonomics. Re-exported here: `MessageRole`, `Citation`.
//!
//! - **Hand-written types**: Used when:
//!   1. **Different Rust structure needed**: `ContentPart`, `InputItem`, `OutputItem` are
//!      enums in SDK but structs in OpenAPI. Enums provide better pattern matching ergonomics.
//!   2. **Stronger typing**: `RunsCreateResponse`/`RunsGetResponse` use `RunId`/`PlanHash`
//!      newtypes instead of raw `Uuid`/`String` for type safety.
//!   3. **Ergonomic factory methods**: `Tool`, `ToolChoice`, `Usage` have builder-style APIs
//!      that aren't generated.
//!
//! Both approaches produce identical JSON serialization, ensuring wire compatibility.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::errors::{Error, ValidationError};
use crate::identifiers::{ProviderId, TierCode};

// Re-export from generated module (single source of truth)
pub use crate::generated::{Citation, MessageRole, MessageRoleExt};

/// Stop reason returned by the backend and surfaced by `/responses`.
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

// MessageRole is now imported from crate::generated
// Extension trait MessageRoleExt provides the as_str() method

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

/// Content part within a message (currently only `text`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }
}

/// Input item sent to `/responses`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message {
        role: MessageRole,
        content: Vec<ContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
    },
}

impl InputItem {
    pub fn message(role: MessageRole, content: impl Into<String>) -> Self {
        InputItem::Message {
            role,
            content: vec![ContentPart::text(content)],
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::message(MessageRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::message(MessageRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::message(MessageRole::Assistant, content)
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        InputItem::Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::text(content)],
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Output item returned by `/responses`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message {
        role: MessageRole,
        content: Vec<ContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
    },
}

/// Output format configuration (structured outputs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputFormat {
    #[serde(rename = "type")]
    pub kind: OutputFormatKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<JSONSchemaFormat>,
}

impl OutputFormat {
    pub fn text() -> Self {
        Self {
            kind: OutputFormatKind::Text,
            json_schema: None,
        }
    }

    pub fn json_schema(name: impl Into<String>, schema: Value) -> Self {
        Self {
            kind: OutputFormatKind::JsonSchema,
            json_schema: Some(JSONSchemaFormat {
                name: name.into(),
                description: None,
                schema,
                strict: Some(true),
            }),
        }
    }

    pub fn json_schema_with_description(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: Value,
    ) -> Self {
        Self {
            kind: OutputFormatKind::JsonSchema,
            json_schema: Some(JSONSchemaFormat {
                name: name.into(),
                description: Some(description.into()),
                schema,
                strict: Some(true),
            }),
        }
    }

    pub fn is_structured(&self) -> bool {
        self.kind == OutputFormatKind::JsonSchema
    }
}

/// Supported output format types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum OutputFormatKind {
    Text,
    JsonSchema,
    Other(String),
}

impl OutputFormatKind {
    pub fn as_str(&self) -> &str {
        match self {
            OutputFormatKind::Text => "text",
            OutputFormatKind::JsonSchema => "json_schema",
            OutputFormatKind::Other(other) => other.as_str(),
        }
    }
}

impl From<&str> for OutputFormatKind {
    fn from(value: &str) -> Self {
        OutputFormatKind::from(value.to_string())
    }
}

impl From<String> for OutputFormatKind {
    fn from(value: String) -> Self {
        let trimmed = value.trim();
        match trimmed.to_lowercase().as_str() {
            "text" => OutputFormatKind::Text,
            "json_schema" => OutputFormatKind::JsonSchema,
            _ => OutputFormatKind::Other(trimmed.to_string()),
        }
    }
}

impl From<OutputFormatKind> for String {
    fn from(value: OutputFormatKind) -> Self {
        value.as_str().to_string()
    }
}

impl fmt::Display for OutputFormatKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// JSON schema payload for structured outputs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JSONSchemaFormat {
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
    pub intent: Option<WebToolIntent>,
}

/// Web tool intent (user-facing).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebToolIntent {
    Auto,
    SearchWeb,
    FetchURL,
}

impl WebToolIntent {
    pub fn as_str(&self) -> &str {
        match self {
            WebToolIntent::Auto => "auto",
            WebToolIntent::SearchWeb => "search_web",
            WebToolIntent::FetchURL => "fetch_url",
        }
    }
}

impl fmt::Display for WebToolIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
}

impl ToolChoice {
    pub fn auto() -> Self {
        Self {
            kind: ToolChoiceType::Auto,
            function: None,
        }
    }

    pub fn required() -> Self {
        Self {
            kind: ToolChoiceType::Required,
            function: None,
        }
    }

    pub fn none() -> Self {
        Self {
            kind: ToolChoiceType::None,
            function: None,
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

/// Request payload for `POST /responses`.
/// This is internal - users should use `ResponseBuilder` instead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ResponseRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider: Option<ProviderId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_format: Option<OutputFormat>,
    pub(crate) input: Vec<InputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_choice: Option<ToolChoice>,
}

impl ResponseRequest {
    pub fn validate(&self, require_model: bool) -> Result<(), Error> {
        if require_model && self.model.as_ref().map(|m| m.is_empty()).unwrap_or(true) {
            return Err(Error::Validation(
                ValidationError::new("model is required").with_field("model"),
            ));
        }
        if self.input.is_empty() {
            return Err(Error::Validation(
                ValidationError::new("at least one input item is required").with_field("input"),
            ));
        }
        if let Some(format) = &self.output_format {
            validate_output_format(format)?;
        }
        Ok(())
    }
}

fn validate_output_format(format: &OutputFormat) -> Result<(), Error> {
    match &format.kind {
        OutputFormatKind::Text => Ok(()),
        OutputFormatKind::JsonSchema => {
            let Some(schema) = &format.json_schema else {
                return Err(Error::Validation(
                    ValidationError::new(
                        "output_format.json_schema required when type=json_schema",
                    )
                    .with_field("output_format.json_schema"),
                ));
            };

            if schema.name.trim().is_empty() {
                return Err(Error::Validation(
                    ValidationError::new("output_format.json_schema.name required")
                        .with_field("output_format.json_schema.name"),
                ));
            }
            if schema.schema.is_null() {
                return Err(Error::Validation(
                    ValidationError::new("output_format.json_schema.schema required")
                        .with_field("output_format.json_schema.schema"),
                ));
            }
            if !schema.schema.is_object() {
                return Err(Error::Validation(
                    ValidationError::new("output_format.json_schema.schema must be an object")
                        .with_field("output_format.json_schema.schema"),
                ));
            }
            Ok(())
        }
        OutputFormatKind::Other(other) => Err(Error::Validation(
            ValidationError::new(format!("invalid output_format.type: {}", other))
                .with_field("output_format.type"),
        )),
    }
}

/// Aggregated response returned by `POST /responses`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    pub model: Model,
    pub output: Vec<OutputItem>,
    pub usage: Usage,
    /// Request identifier echoed by the API (response header).
    #[serde(default, skip_serializing)]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<Citation>>,
}

impl Response {
    /// Returns the concatenated assistant text output, in order.
    ///
    /// This is a client-side convenience that walks `response.output` and:
    /// - includes only output items where `role == assistant`
    /// - includes only `text` content parts
    ///
    /// If the response contains no assistant text, this returns an empty string.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for item in &self.output {
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
                    ContentPart::Text { text } => out.push_str(text),
                }
            }
        }
        out
    }

    /// Returns the assistant text content parts as individual chunks, in order.
    ///
    /// This is useful when you want to preserve message/content boundaries instead of
    /// joining everything together.
    pub fn text_chunks(&self) -> Vec<String> {
        let mut out = Vec::new();
        for item in &self.output {
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
                    ContentPart::Text { text } => out.push(text.clone()),
                }
            }
        }
        out
    }
}

// Citation is now imported from crate::generated (identical structure)

/// Token usage metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsageSummary {
    pub plan: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub window_start: Option<DateTime<Utc>>,
    #[serde(default)]
    pub window_end: Option<DateTime<Utc>>,
    #[serde(default)]
    pub limit: u64,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    pub images: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions_used: Option<u64>,
    #[serde(default)]
    pub remaining: u64,
    #[serde(default)]
    pub state: String,
}

impl Usage {
    /// Prompt tokens counted by the backend.
    pub fn input(&self) -> u64 {
        self.input_tokens
    }

    /// Completion tokens counted by the backend.
    pub fn output(&self) -> u64 {
        self.output_tokens
    }

    /// Total tokens (computed from input/output if the field was omitted).
    pub fn total(&self) -> u64 {
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
            "start" => Self::MessageStart,
            "update" => Self::MessageDelta,
            "completion" => Self::MessageStop,
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
    pub index: u32,
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
    /// Tool result payload when kind is ToolUseStop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<serde_json::Value>,
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
    #[serde(rename = "created_at")]
    pub created_at: DateTime<Utc>,
    #[serde(
        default,
        rename = "expires_at",
        skip_serializing_if = "Option::is_none"
    )]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        rename = "last_used_at",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(rename = "redacted_key")]
    pub redacted_key: String,
    #[serde(
        default,
        rename = "secret_key",
        skip_serializing_if = "Option::is_none"
    )]
    pub secret_key: Option<String>,
}

/// Request payload for POST /auth/customer-token.
///
/// Requires secret key auth (mr_sk_*). Exactly one of customer_id or
/// customer_external_id must be provided.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomerTokenRequest {
    #[serde(skip_serializing_if = "Option::is_none", rename = "customer_id")]
    pub customer_id: Option<Uuid>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "customer_external_id"
    )]
    pub customer_external_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "ttl_seconds")]
    pub ttl_seconds: Option<u32>,
}

impl CustomerTokenRequest {
    pub fn for_customer_id(customer_id: Uuid) -> Self {
        Self {
            customer_id: Some(customer_id),
            customer_external_id: None,
            ttl_seconds: None,
        }
    }

    pub fn for_external_id(customer_external_id: impl Into<String>) -> Self {
        Self {
            customer_id: None,
            customer_external_id: Some(customer_external_id.into()),
            ttl_seconds: None,
        }
    }

    pub fn with_ttl_seconds(mut self, ttl: u32) -> Self {
        self.ttl_seconds = Some(ttl);
        self
    }
}

/// Short-lived bearer token usable from browser/mobile clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomerToken {
    /// The bearer token for authenticating LLM requests.
    pub token: String,
    /// When the token expires.
    pub expires_at: DateTime<Utc>,
    /// Seconds until the token expires (also computable from expires_at).
    pub expires_in: u32,
    /// The project ID this token is scoped to.
    pub project_id: Uuid,
    /// The internal customer ID (UUID).
    /// Optional for BYOB (external billing) projects where customers are not created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<Uuid>,
    /// The external customer ID, when present.
    pub customer_external_id: String,
    /// The tier code for the customer (e.g., "free", "pro", "enterprise").
    /// Optional for BYOB (external billing) projects where customers may not have subscriptions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_code: Option<TierCode>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Device Authorization Flow (RFC 8628)
// ─────────────────────────────────────────────────────────────────────────────

/// Provider for native device authorization flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceFlowProvider {
    /// Use GitHub's native device flow (RFC 8628).
    /// User authenticates directly at github.com/login/device.
    Github,
}

/// Request options for starting a device authorization flow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceStartRequest {
    /// Provider for native device flow. Leave empty for wrapped OAuth flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<DeviceFlowProvider>,
}

// Note: DeviceStartResponse is generated from OpenAPI spec.
// Use `crate::generated::DeviceStartResponse`.

/// OAuth provider for customer authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProvider {
    /// GitHub OAuth provider.
    Github,
    /// Google OAuth provider.
    Google,
}

/// Request options for starting an OAuth redirect flow for customer authentication.
///
/// This initiates a standard OAuth 2.0 authorization code flow where users
/// authenticate with GitHub or Google and are redirected back to your application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthStartRequest {
    /// The project ID to authenticate against.
    pub project_id: Uuid,
    /// The OAuth provider to use: "github" or "google".
    pub provider: OAuthProvider,
    /// Where to redirect after OAuth completion.
    /// Must be whitelisted in the project's OAuth redirect URIs.
    pub redirect_uri: String,
}

/// Response from starting an OAuth redirect flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthStartResponse {
    /// The URL to redirect the user to for OAuth authentication.
    pub redirect_url: String,
}

/// Typed error kinds for device flow failures.
///
/// RFC 8628 defines standard error codes that can occur during device authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceFlowErrorKind {
    /// The authorization request is still pending; the user hasn't completed authorization yet.
    /// Keep polling at the specified interval.
    AuthorizationPending,
    /// Polling too frequently. Increase the polling interval.
    SlowDown,
    /// The device code has expired. Start a new authorization flow.
    ExpiredToken,
    /// The user denied the authorization request.
    AccessDenied,
    /// Other error not covered by standard codes.
    Other(String),
}

impl DeviceFlowErrorKind {
    /// Parse an OAuth error code string into a typed variant.
    pub fn from_code(code: &str) -> Self {
        match code {
            "authorization_pending" => Self::AuthorizationPending,
            "slow_down" => Self::SlowDown,
            "expired_token" => Self::ExpiredToken,
            "access_denied" => Self::AccessDenied,
            other => Self::Other(other.to_string()),
        }
    }

    /// Returns true if this error indicates the user should keep polling.
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::AuthorizationPending | Self::SlowDown)
    }

    /// Returns true if this error is terminal (no point in continuing to poll).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::ExpiredToken | Self::AccessDenied)
    }
}

impl std::fmt::Display for DeviceFlowErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthorizationPending => write!(f, "authorization_pending"),
            Self::SlowDown => write!(f, "slow_down"),
            Self::ExpiredToken => write!(f, "expired_token"),
            Self::AccessDenied => write!(f, "access_denied"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

/// Successful token response from device flow.
///
/// Uses `chrono::DateTime<Utc>` for expiration timestamps, consistent with the rest of the SDK.
#[derive(Debug, Clone)]
pub struct DeviceTokenResponse {
    /// The customer bearer token.
    pub token: String,
    /// When the token expires.
    pub expires_at: DateTime<Utc>,
    /// Seconds until expiration.
    pub expires_in: u32,
    /// The project ID this token is scoped to.
    pub project_id: Uuid,
    /// The customer ID this token represents.
    /// Optional for BYOB (external billing) projects where customers are not created.
    pub customer_id: Option<Uuid>,
    /// The customer's external ID (if set).
    pub customer_external_id: String,
    /// The customer's tier code.
    /// Optional for BYOB (external billing) projects where customers may not have subscriptions.
    pub tier_code: Option<String>,
}

/// Pending state when user hasn't completed authorization yet.
#[derive(Debug, Clone)]
pub struct DeviceTokenPending {
    /// The error kind (AuthorizationPending or SlowDown).
    pub kind: DeviceFlowErrorKind,
    /// Optional human-readable error message.
    pub description: Option<String>,
    /// Updated polling interval in seconds (present when kind is SlowDown).
    pub interval: Option<u32>,
}

/// Result of polling the device token endpoint.
///
/// This is a discriminated union - check the variant to determine the outcome:
/// - `Approved`: User authorized, token is available
/// - `Pending`: User hasn't completed authorization yet, keep polling
/// - `Error`: Authorization failed (expired, denied, etc.)
#[derive(Debug, Clone)]
pub enum DeviceTokenResult {
    /// User authorized, token is available.
    Approved(DeviceTokenResponse),
    /// User hasn't completed authorization yet. Check `kind` for details.
    Pending(DeviceTokenPending),
    /// Authorization failed (expired, denied, etc.).
    Error {
        /// The typed error kind.
        kind: DeviceFlowErrorKind,
        /// Optional human-readable error description.
        description: Option<String>,
    },
}

/// Subscription status for billing integrations.
///
/// Maps to Stripe's subscription statuses with forward-compatible `Unknown` variant.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatusKind {
    Active,
    PastDue,
    Canceled,
    Trialing,
    Incomplete,
    IncompleteExpired,
    Unpaid,
    Paused,
    /// Unknown status for forward compatibility with new Stripe statuses.
    #[serde(other)]
    Unknown,
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
    fn responses_request_validation_guards_required_fields() {
        // Empty model fails validation when required
        let req = ResponseRequest {
            provider: None,
            model: Some(Model::from("")),
            input: vec![InputItem::user("hi")],
            max_output_tokens: None,
            temperature: None,
            output_format: None,
            stop: None,
            tools: None,
            tool_choice: None,
        };
        assert!(req.validate(true).is_err());

        // Empty input fails validation
        let req = ResponseRequest {
            provider: None,
            model: Some(Model::from("gpt-4o-mini")),
            input: Vec::new(),
            max_output_tokens: None,
            temperature: None,
            output_format: None,
            stop: None,
            tools: None,
            tool_choice: None,
        };
        assert!(req.validate(true).is_err());

        // Valid request passes validation
        let req = ResponseRequest {
            provider: None,
            model: Some(Model::from("gpt-4o-mini")),
            input: vec![InputItem::user("hi")],
            max_output_tokens: None,
            temperature: None,
            output_format: None,
            stop: None,
            tools: None,
            tool_choice: None,
        };
        assert!(req.validate(true).is_ok());
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(
            json.get("model").and_then(|v| v.as_str()),
            Some("gpt-4o-mini")
        );
    }

    fn response_with_output(output: Vec<OutputItem>) -> Response {
        Response {
            id: "resp_test".to_string(),
            stop_reason: None,
            model: Model::from("test-model"),
            output,
            usage: Usage::default(),
            request_id: None,
            provider: None,
            citations: None,
        }
    }

    #[test]
    fn response_text_empty_output() {
        let response = response_with_output(Vec::new());
        assert_eq!(response.text(), "");
        assert_eq!(response.text_chunks(), Vec::<String>::new());
    }

    #[test]
    fn response_text_ignores_non_assistant_messages() {
        let response = response_with_output(vec![
            OutputItem::Message {
                role: MessageRole::User,
                content: vec![ContentPart::text("user text")],
                tool_calls: None,
            },
            OutputItem::Message {
                role: MessageRole::Tool,
                content: vec![ContentPart::text("tool output")],
                tool_calls: None,
            },
        ]);
        assert_eq!(response.text(), "");
        assert_eq!(response.text_chunks(), Vec::<String>::new());
    }

    #[test]
    fn response_text_preserves_order_across_assistant_output_items() {
        let response = response_with_output(vec![
            OutputItem::Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::text("a")],
                tool_calls: None,
            },
            OutputItem::Message {
                role: MessageRole::System,
                content: vec![ContentPart::text("ignored")],
                tool_calls: None,
            },
            OutputItem::Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::text("b")],
                tool_calls: None,
            },
        ]);
        assert_eq!(
            response.text_chunks(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(response.text(), "ab");
    }

    #[test]
    fn response_text_preserves_order_across_multiple_text_parts() {
        let response = response_with_output(vec![OutputItem::Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::text("hello "), ContentPart::text("world")],
            tool_calls: None,
        }]);
        assert_eq!(
            response.text_chunks(),
            vec!["hello ".to_string(), "world".to_string()]
        );
        assert_eq!(response.text(), "hello world");
    }

    #[test]
    fn response_text_tool_call_only_messages_yield_empty() {
        let response = response_with_output(vec![OutputItem::Message {
            role: MessageRole::Assistant,
            content: Vec::new(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                kind: ToolType::Function,
                function: None,
            }]),
        }]);
        assert_eq!(response.text(), "");
        assert_eq!(response.text_chunks(), Vec::<String>::new());
    }
}
