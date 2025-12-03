use std::{collections::HashMap, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::errors::{Error, ValidationError};

/// Stop reason returned by providers and surfaced by `/llm/proxy`.
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

/// Known provider identifiers with an escape hatch for custom IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Provider {
    OpenAI,
    Anthropic,
    Grok,
    Echo,
    Other(String),
}

impl Provider {
    pub fn as_str(&self) -> &str {
        match self {
            Provider::OpenAI => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Grok => "grok",
            Provider::Echo => "echo",
            Provider::Other(other) => other.as_str(),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Provider::Other(other) if other.trim().is_empty())
    }
}

impl From<&str> for Provider {
    fn from(value: &str) -> Self {
        Provider::from(value.to_string())
    }
}

impl From<String> for Provider {
    fn from(value: String) -> Self {
        let trimmed = value.trim();
        match trimmed.to_lowercase().as_str() {
            "openai" => Provider::OpenAI,
            "anthropic" => Provider::Anthropic,
            "grok" => Provider::Grok,
            "echo" => Provider::Echo,
            _ => Provider::Other(trimmed.to_string()),
        }
    }
}

impl From<Provider> for String {
    fn from(value: Provider) -> Self {
        value.as_str().to_string()
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Common model identifiers with `Other` for custom/preview models.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Model {
    OpenAIGpt4o,
    OpenAIGpt4oMini,
    AnthropicClaude35HaikuLatest,
    AnthropicClaude35SonnetLatest,
    AnthropicClaudeOpus45,
    AnthropicClaude35Haiku,
    Grok2,
    Grok4_1FastNonReasoning,
    Grok4_1FastReasoning,
    Echo1,
    Other(String),
}

impl Model {
    pub fn as_str(&self) -> &str {
        match self {
            Model::OpenAIGpt4o => "openai/gpt-4o",
            Model::OpenAIGpt4oMini => "openai/gpt-4o-mini",
            Model::AnthropicClaude35HaikuLatest => "anthropic/claude-3-5-haiku-latest",
            Model::AnthropicClaude35SonnetLatest => "anthropic/claude-3-5-sonnet-latest",
            Model::AnthropicClaudeOpus45 => "anthropic/claude-opus-4-5-20251101",
            Model::AnthropicClaude35Haiku => "anthropic/claude-3.5-haiku",
            Model::Grok2 => "grok-2",
            Model::Grok4_1FastNonReasoning => "grok-4-1-fast-non-reasoning",
            Model::Grok4_1FastReasoning => "grok-4-1-fast-reasoning",
            Model::Echo1 => "echo-1",
            Model::Other(other) => other.as_str(),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Model::Other(other) if other.trim().is_empty())
    }
}

impl From<&str> for Model {
    fn from(value: &str) -> Self {
        Model::from(value.to_string())
    }
}

impl From<String> for Model {
    fn from(value: String) -> Self {
        let trimmed = value.trim();
        match trimmed.to_lowercase().as_str() {
            "openai/gpt-4o" => Model::OpenAIGpt4o,
            "openai/gpt-4o-mini" => Model::OpenAIGpt4oMini,
            "anthropic/claude-3-5-haiku-latest" => Model::AnthropicClaude35HaikuLatest,
            "anthropic/claude-3-5-sonnet-latest" => Model::AnthropicClaude35SonnetLatest,
            "anthropic/claude-opus-4-5-20251101" => Model::AnthropicClaudeOpus45,
            "anthropic/claude-3.5-haiku" => Model::AnthropicClaude35Haiku,
            "grok-2" => Model::Grok2,
            "grok-4-1-fast-non-reasoning" => Model::Grok4_1FastNonReasoning,
            "grok-4-1-fast-reasoning" => Model::Grok4_1FastReasoning,
            "echo-1" => Model::Echo1,
            _ => Model::Other(trimmed.to_string()),
        }
    }
}

impl From<Model> for String {
    fn from(value: Model) -> Self {
        value.as_str().to_string()
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
    pub role: String,
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
    pub fn is_structured(&self) -> bool {
        matches!(
            self.kind,
            ResponseFormatKind::JsonObject | ResponseFormatKind::JsonSchema
        )
    }
}

/// Supported response format types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ResponseFormatKind {
    Text,
    JsonObject,
    JsonSchema,
    Other(String),
}

impl ResponseFormatKind {
    pub fn as_str(&self) -> &str {
        match self {
            ResponseFormatKind::Text => "text",
            ResponseFormatKind::JsonObject => "json_object",
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
            "json_object" => ResponseFormatKind::JsonObject,
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
    WebSearch,
    XSearch,
    CodeExecution,
}

impl ToolType {
    pub fn as_str(&self) -> &str {
        match self {
            ToolType::Function => "function",
            ToolType::WebSearch => "web_search",
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

/// Web search configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WebSearchConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<i32>,
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
    pub web_search: Option<WebSearchConfig>,
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
            web_search: None,
            x_search: None,
            code_execution: None,
        }
    }

    /// Create a web search tool.
    pub fn web_search(config: WebSearchConfig) -> Self {
        Self {
            kind: ToolType::WebSearch,
            function: None,
            web_search: Some(config),
            x_search: None,
            code_execution: None,
        }
    }

    /// Create an X/Twitter search tool.
    pub fn x_search(config: XSearchConfig) -> Self {
        Self {
            kind: ToolType::XSearch,
            function: None,
            web_search: None,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    pub model: Model,
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
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "stop_sequences",
        alias = "stopSequences"
    )]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
}

impl ProxyRequest {
    pub fn new(model: impl Into<Model>, messages: Vec<ProxyMessage>) -> Result<Self, Error> {
        let req = Self {
            provider: None,
            model: model.into(),
            max_tokens: None,
            temperature: None,
            messages,
            metadata: None,
            response_format: None,
            stop: None,
            stop_sequences: None,
            tools: None,
            tool_choice: None,
        };
        req.validate()?;
        Ok(req)
    }

    /// Build a proxy request with fluent setters and validation.
    pub fn builder(model: impl Into<Model>) -> ProxyRequestBuilder {
        ProxyRequestBuilder::new(model)
    }

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
        ResponseFormatKind::JsonObject | ResponseFormatKind::Text => Ok(()),
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

/// Fluent builder for [`ProxyRequest`].
#[derive(Debug, Clone)]
pub struct ProxyRequestBuilder {
    provider: Option<Provider>,
    model: Model,
    max_tokens: Option<i64>,
    temperature: Option<f64>,
    messages: Vec<ProxyMessage>,
    metadata: Option<HashMap<String, String>>,
    response_format: Option<ResponseFormat>,
    stop: Option<Vec<String>>,
    stop_sequences: Option<Vec<String>>,
    tools: Option<Vec<Tool>>,
    tool_choice: Option<ToolChoice>,
}

impl ProxyRequestBuilder {
    pub fn new(model: impl Into<Model>) -> Self {
        Self {
            model: model.into(),
            provider: None,
            max_tokens: None,
            temperature: None,
            messages: Vec::new(),
            metadata: None,
            response_format: None,
            stop: None,
            stop_sequences: None,
            tools: None,
            tool_choice: None,
        }
    }

    pub fn provider(mut self, provider: impl Into<Provider>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn max_tokens(mut self, max_tokens: i64) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn message(mut self, role: impl Into<String>, content: impl Into<String>) -> Self {
        self.messages.push(ProxyMessage {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        });
        self
    }

    pub fn system(self, content: impl Into<String>) -> Self {
        self.message("system", content)
    }

    pub fn user(self, content: impl Into<String>) -> Self {
        self.message("user", content)
    }

    pub fn assistant(self, content: impl Into<String>) -> Self {
        self.message("assistant", content)
    }

    pub fn messages(mut self, messages: Vec<ProxyMessage>) -> Self {
        self.messages = messages;
        self
    }

    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn metadata_entry(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
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

    pub fn response_format(mut self, response_format: ResponseFormat) -> Self {
        self.response_format = Some(response_format);
        self
    }

    pub fn stop(mut self, stop: Vec<String>) -> Self {
        self.stop = Some(stop);
        self
    }

    pub fn stop_sequences(mut self, stop_sequences: Vec<String>) -> Self {
        self.stop_sequences = Some(stop_sequences);
        self
    }

    pub fn tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn tool(mut self, tool: Tool) -> Self {
        let mut tools = self.tools.unwrap_or_default();
        tools.push(tool);
        self.tools = Some(tools);
        self
    }

    pub fn function_tool(
        self,
        name: impl Into<String>,
        description: Option<String>,
        parameters: Option<Value>,
    ) -> Self {
        self.tool(Tool::function(name, description, parameters))
    }

    pub fn tool_choice(mut self, tool_choice: ToolChoice) -> Self {
        self.tool_choice = Some(tool_choice);
        self
    }

    pub fn tool_choice_auto(self) -> Self {
        self.tool_choice(ToolChoice::auto())
    }

    pub fn tool_choice_required(self) -> Self {
        self.tool_choice(ToolChoice::required())
    }

    pub fn tool_choice_none(self) -> Self {
        self.tool_choice(ToolChoice::none())
    }

    pub fn build(self) -> Result<ProxyRequest, Error> {
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
        if !self
            .messages
            .iter()
            .any(|msg| msg.role.eq_ignore_ascii_case("user"))
        {
            return Err(Error::Validation(
                ValidationError::new("at least one user message is required")
                    .with_field("messages"),
            ));
        }
        if let Some(provider) = &self.provider {
            if provider.is_empty() {
                return Err(Error::Validation(
                    ValidationError::new("provider is required").with_field("provider"),
                ));
            }
        }

        let req = ProxyRequest {
            provider: self.provider,
            model: self.model,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            messages: self.messages,
            metadata: self.metadata,
            response_format: self.response_format,
            stop: self.stop,
            stop_sequences: self.stop_sequences,
            tools: self.tools,
            tool_choice: self.tool_choice,
        };
        req.validate()?;
        Ok(req)
    }
}

/// Aggregated response returned by `/llm/proxy`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyResponse {
    pub provider: Provider,
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
    /// Prompt tokens counted by the provider.
    pub fn input(&self) -> i64 {
        self.input_tokens
    }

    /// Completion tokens counted by the provider.
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

/// Request payload for POST /auth/frontend-token.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FrontendTokenRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publishable_key: Option<String>,
    #[serde(rename = "customer_id", skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "device_id")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "ttl_seconds")]
    pub ttl_seconds: Option<i64>,
}

/// Short-lived bearer token usable from browser/mobile clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrontendToken {
    pub token: String,
    #[serde(
        default,
        rename = "expires_at",
        alias = "expiresAt",
        with = "time::serde::rfc3339::option"
    )]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(default, rename = "expires_in", alias = "expiresIn")]
    pub expires_in: Option<u32>,
    #[serde(default, rename = "token_type", alias = "tokenType")]
    pub token_type: Option<String>,
    #[serde(default, rename = "key_id", alias = "keyId")]
    pub key_id: Option<Uuid>,
    #[serde(default, rename = "session_id", alias = "sessionId")]
    pub session_id: Option<Uuid>,
    #[serde(default, rename = "token_scope", alias = "tokenScope")]
    pub token_scope: Option<Vec<String>>,
    #[serde(default, rename = "token_source", alias = "tokenSource")]
    pub token_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
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
    fn provider_and_model_enums_capture_custom_values() {
        let provider: Provider = serde_json::from_str("\"openai\"").unwrap();
        assert_eq!(provider, Provider::OpenAI);
        let custom_provider: Provider = serde_json::from_str("\"acme\"").unwrap();
        assert!(matches!(custom_provider, Provider::Other(val) if val == "acme"));

        let model: Model = serde_json::from_str("\"openai/gpt-4o-mini\"").unwrap();
        assert_eq!(model, Model::OpenAIGpt4oMini);
        let other_model: Model = serde_json::from_str("\"my/model\"").unwrap();
        assert!(matches!(other_model, Model::Other(val) if val == "my/model"));
    }

    #[test]
    fn proxy_request_validation_guards_required_fields() {
        let err = ProxyRequest::new("openai/gpt-4o-mini", Vec::new()).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));

        let req = ProxyRequest::new(
            Model::OpenAIGpt4oMini,
            vec![ProxyMessage {
                role: "user".into(),
                content: "hi".into(),
                tool_calls: None,
                tool_call_id: None,
            }],
        )
        .unwrap();
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(
            json.get("model").and_then(|v| v.as_str()),
            Some("openai/gpt-4o-mini")
        );
    }

    #[test]
    fn proxy_request_builder_populates_fields() {
        let req = ProxyRequest::builder(Model::OpenAIGpt4oMini)
            .provider(Provider::OpenAI)
            .system("You are helpful.")
            .user("hi")
            .assistant("hello")
            .max_tokens(256)
            .temperature(0.3)
            .metadata_entry("trace_id", "abc123")
            .stop(vec!["stop".into()])
            .stop_sequences(vec!["stopseq".into()])
            .build()
            .unwrap();

        assert_eq!(req.provider, Some(Provider::OpenAI));
        assert_eq!(req.messages.len(), 3);
        assert_eq!(req.max_tokens, Some(256));
        assert_eq!(req.temperature, Some(0.3));
        assert_eq!(
            req.metadata
                .as_ref()
                .and_then(|m| m.get("trace_id"))
                .cloned(),
            Some("abc123".into())
        );
        assert_eq!(req.stop.as_ref().map(|s| s.len()), Some(1));
        assert_eq!(req.stop_sequences.as_ref().map(|s| s.len()), Some(1));
    }

    #[test]
    fn proxy_request_builder_requires_user_message() {
        let err = ProxyRequest::builder("openai/gpt-4o-mini")
            .system("hi")
            .build()
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }
}
