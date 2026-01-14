//! Tool helpers for ergonomic tool use with the ModelRelay API.

use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{
    ContentPart, FunctionCall, FunctionTool, InputItem, MessageRole, OutputItem, Response, Tool,
    ToolCall, ToolCallDelta, ToolType,
};

/// tools.v0 user_ask tool name.
pub const USER_ASK_TOOL_NAME: &str = "user_ask";

/// User ask option for user.ask.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserAskOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Arguments for the user.ask tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserAskArgs {
    pub question: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub options: Vec<UserAskOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_freeform: Option<bool>,
}

/// Response for the user.ask tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserAskResponse {
    pub answer: String,
    pub is_freeform: bool,
}

/// Extension trait for `Response` with tool-related convenience methods.
pub trait ResponseExt {
    /// Returns true if the response contains tool calls.
    fn has_tool_calls(&self) -> bool;

    /// Returns the first tool call, or None if there are no tool calls.
    fn first_tool_call(&self) -> Option<&ToolCall>;
}

impl ResponseExt for Response {
    fn has_tool_calls(&self) -> bool {
        self.output.iter().any(|item| match item {
            OutputItem::Message { tool_calls, .. } => {
                tool_calls.as_ref().is_some_and(|t| !t.is_empty())
            }
        })
    }

    fn first_tool_call(&self) -> Option<&ToolCall> {
        for item in &self.output {
            if let OutputItem::Message {
                tool_calls: Some(tool_calls),
                ..
            } = item
            {
                if let Some(first) = tool_calls.first() {
                    return Some(first);
                }
            }
        }
        None
    }
}

/// Creates a message containing the result of a tool call.
pub fn tool_result_message(
    tool_call_id: impl Into<String>,
    result: impl Into<String>,
) -> InputItem {
    InputItem::tool_result(tool_call_id, result)
}

/// Creates a tool result message with JSON serialization.
pub fn tool_result_message_json<T: serde::Serialize>(
    tool_call_id: impl Into<String>,
    result: &T,
) -> Result<InputItem, serde_json::Error> {
    let content = serde_json::to_string(result)?;
    Ok(tool_result_message(tool_call_id, content))
}

/// Creates a tool result message from a ToolCall.
pub fn respond_to_tool_call(call: &ToolCall, result: impl Into<String>) -> InputItem {
    tool_result_message(&call.id, result)
}

/// Creates a tool result message from a ToolCall with JSON serialization.
pub fn respond_to_tool_call_json<T: serde::Serialize>(
    call: &ToolCall,
    result: &T,
) -> Result<InputItem, serde_json::Error> {
    tool_result_message_json(&call.id, result)
}

/// Returns the tools.v0 definition for the user.ask tool.
pub fn user_ask_tool() -> Tool {
    Tool::function(
        USER_ASK_TOOL_NAME,
        Some("Ask the user a clarifying question.".to_string()),
        Some(user_ask_schema()),
    )
}

/// Returns true if the tool call is user.ask.
pub fn is_user_ask_tool_call(call: &ToolCall) -> bool {
    call.kind == ToolType::Function
        && call
            .function
            .as_ref()
            .is_some_and(|f| f.name == USER_ASK_TOOL_NAME)
}

/// Parses and validates user.ask arguments.
pub fn parse_user_ask_args(call: &ToolCall) -> Result<UserAskArgs, ToolArgsError> {
    parse_and_validate_tool_args(call)
}

/// Serializes a user.ask response as JSON.
pub fn serialize_user_ask_result(result: &UserAskResponse) -> Result<String, ToolArgsError> {
    if result.answer.trim().is_empty() {
        return Err(ToolArgsError {
            message: "user_ask answer is required".to_string(),
            tool_call_id: "".to_string(),
            tool_name: USER_ASK_TOOL_NAME.to_string(),
            raw_arguments: "".to_string(),
        });
    }
    serde_json::to_string(result).map_err(|e| ToolArgsError {
        message: format!("failed to serialize user.ask result: {e}"),
        tool_call_id: "".to_string(),
        tool_name: USER_ASK_TOOL_NAME.to_string(),
        raw_arguments: "".to_string(),
    })
}

/// Builds a freeform user.ask result string.
pub fn user_ask_result_freeform(answer: impl Into<String>) -> Result<String, ToolArgsError> {
    serialize_user_ask_result(&UserAskResponse {
        answer: answer.into(),
        is_freeform: true,
    })
}

/// Builds a multiple-choice user.ask result string.
pub fn user_ask_result_choice(answer: impl Into<String>) -> Result<String, ToolArgsError> {
    serialize_user_ask_result(&UserAskResponse {
        answer: answer.into(),
        is_freeform: false,
    })
}

fn user_ask_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "question": {
                "type": "string",
                "minLength": 1,
                "description": "The question to ask the user."
            },
            "options": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string", "minLength": 1 },
                        "description": { "type": "string" }
                    },
                    "required": ["label"]
                },
                "description": "Optional multiple choice options."
            },
            "allow_freeform": {
                "type": "boolean",
                "default": true,
                "description": "Allow user to type a custom response."
            }
        },
        "required": ["question"]
    })
}

/// Creates an assistant message that includes tool calls.
pub fn assistant_message_with_tool_calls(
    content: impl Into<String>,
    tool_calls: Vec<ToolCall>,
) -> InputItem {
    InputItem::Message {
        role: MessageRole::Assistant,
        content: vec![ContentPart::text(content)],
        tool_calls: Some(tool_calls),
        tool_call_id: None,
    }
}

/// Accumulates streaming tool call deltas into complete tool calls.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: HashMap<u32, ToolCall>,
}

impl ToolCallAccumulator {
    /// Creates a new accumulator for streaming tool calls.
    pub fn new() -> Self {
        Self {
            calls: HashMap::new(),
        }
    }

    /// Processes a streaming tool call delta.
    /// Returns true if this started a new tool call.
    pub fn process_delta(&mut self, delta: &ToolCallDelta) -> bool {
        if let Some(existing) = self.calls.get_mut(&delta.index) {
            // Append to existing tool call
            if let Some(ref func_delta) = delta.function {
                if let Some(ref mut func) = existing.function {
                    if let Some(ref name) = func_delta.name {
                        func.name = name.clone();
                    }
                    if let Some(ref args) = func_delta.arguments {
                        func.arguments.push_str(args);
                    }
                }
            }
            false
        } else {
            // New tool call
            let function = delta.function.as_ref().map(|f| FunctionCall {
                name: f.name.clone().unwrap_or_default(),
                arguments: f.arguments.clone().unwrap_or_default(),
            });

            self.calls.insert(
                delta.index,
                ToolCall {
                    id: delta.id.clone().unwrap_or_default(),
                    kind: delta
                        .type_
                        .as_ref()
                        .map(|t| match t.as_str() {
                            "function" => ToolType::Function,
                            "x_search" => ToolType::XSearch,
                            "code_execution" => ToolType::CodeExecution,
                            _ => ToolType::Function,
                        })
                        .unwrap_or(ToolType::Function),
                    function,
                },
            );
            true
        }
    }

    /// Returns all accumulated tool calls in index order.
    pub fn get_tool_calls(&self) -> Vec<ToolCall> {
        if self.calls.is_empty() {
            return Vec::new();
        }

        let max_idx = self.calls.keys().max().copied().unwrap_or(0);
        let mut result = Vec::with_capacity(self.calls.len());
        for i in 0..=max_idx {
            if let Some(call) = self.calls.get(&i) {
                result.push(call.clone());
            }
        }
        result
    }

    /// Returns a specific tool call by index, or None if not found.
    pub fn get_tool_call(&self, index: u32) -> Option<&ToolCall> {
        self.calls.get(&index)
    }

    /// Clears all accumulated tool calls.
    pub fn reset(&mut self) {
        self.calls.clear();
    }
}

// ============================================================================
// Type-safe Argument Parsing
// ============================================================================

/// Error returned when tool argument parsing or validation fails.
/// Contains a descriptive message suitable for sending back to the model.
#[derive(Debug, Clone)]
pub struct ToolArgsError {
    /// Human-readable error message
    pub message: String,
    /// The tool call ID for correlation
    pub tool_call_id: String,
    /// The tool name that was called
    pub tool_name: String,
    /// The raw arguments string that failed to parse
    pub raw_arguments: String,
}

impl std::fmt::Display for ToolArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ToolArgsError {}

/// Parses and deserializes tool call arguments into the specified type.
///
/// Uses serde for deserialization, so the target type must implement `DeserializeOwned`.
/// Returns a descriptive error if JSON parsing or deserialization fails.
///
/// # Example
///
/// ```ignore
/// use serde::Deserialize;
/// use modelrelay::parse_tool_args;
///
/// #[derive(Deserialize)]
/// struct WeatherArgs {
///     location: String,
///     #[serde(default = "default_unit")]
///     unit: String,
/// }
///
/// fn default_unit() -> String { "celsius".to_string() }
///
/// // In your tool handler:
/// let args: WeatherArgs = parse_tool_args(&tool_call)?;
/// println!("Location: {}", args.location);
/// ```
pub fn parse_tool_args<T>(call: &ToolCall) -> Result<T, ToolArgsError>
where
    T: serde::de::DeserializeOwned,
{
    let tool_name = call
        .function
        .as_ref()
        .map(|f| f.name.clone())
        .unwrap_or_default();
    let raw_args = call
        .function
        .as_ref()
        .map(|f| f.arguments.clone())
        .unwrap_or_default();

    // Handle empty arguments
    let json_str = if raw_args.is_empty() { "{}" } else { &raw_args };

    serde_json::from_str(json_str).map_err(|e| ToolArgsError {
        message: format!("failed to parse arguments for tool '{}': {}", tool_name, e),
        tool_call_id: call.id.clone(),
        tool_name,
        raw_arguments: raw_args,
    })
}

/// Result type for try_parse_tool_args.
pub type ParseResult<T> = Result<T, ToolArgsError>;

/// Trait for types that can validate themselves after parsing.
///
/// Implement this trait on your args struct for custom validation.
///
/// # Example
///
/// ```ignore
/// use modelrelay::{ValidateArgs, ToolArgsError};
///
/// #[derive(Deserialize)]
/// struct WeatherArgs {
///     location: String,
///     unit: Option<String>,
/// }
///
/// impl ValidateArgs for WeatherArgs {
///     fn validate(&self) -> Result<(), String> {
///         if self.location.is_empty() {
///             return Err("location is required".to_string());
///         }
///         if let Some(unit) = &self.unit {
///             if unit != "celsius" && unit != "fahrenheit" {
///                 return Err(format!("unit must be 'celsius' or 'fahrenheit', got '{}'", unit));
///             }
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait ValidateArgs {
    /// Validates the parsed arguments.
    /// Returns Ok(()) if valid, or an error message if invalid.
    fn validate(&self) -> Result<(), String>;
}

impl ValidateArgs for UserAskArgs {
    fn validate(&self) -> Result<(), String> {
        if self.question.trim().is_empty() {
            return Err("user_ask question is required".to_string());
        }
        for opt in &self.options {
            if opt.label.trim().is_empty() {
                return Err("user_ask option label is required".to_string());
            }
        }
        let allow_freeform = self.allow_freeform.unwrap_or(true);
        if !allow_freeform && self.options.is_empty() {
            return Err("user_ask requires options when allow_freeform=false".to_string());
        }
        Ok(())
    }
}

/// Parses, deserializes, and validates tool call arguments.
///
/// The target type must implement both `DeserializeOwned` and `ValidateArgs`.
///
/// # Example
///
/// ```ignore
/// let args: WeatherArgs = parse_and_validate_tool_args(&tool_call)?;
/// ```
pub fn parse_and_validate_tool_args<T>(call: &ToolCall) -> Result<T, ToolArgsError>
where
    T: serde::de::DeserializeOwned + ValidateArgs,
{
    let args: T = parse_tool_args(call)?;

    args.validate().map_err(|e| {
        let tool_name = call
            .function
            .as_ref()
            .map(|f| f.name.clone())
            .unwrap_or_default();
        let raw_args = call
            .function
            .as_ref()
            .map(|f| f.arguments.clone())
            .unwrap_or_default();

        ToolArgsError {
            message: format!("invalid arguments for tool '{}': {}", tool_name, e),
            tool_call_id: call.id.clone(),
            tool_name,
            raw_arguments: raw_args,
        }
    })?;

    Ok(args)
}

// ============================================================================
// Tool Registry
// ============================================================================

/// Error returned when a tool is not found in the registry.
#[derive(Debug, Clone)]
pub struct UnknownToolError {
    pub tool_name: String,
    pub available: Vec<String>,
}

impl std::fmt::Display for UnknownToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.available.is_empty() {
            write!(
                f,
                "unknown tool: '{}'. No tools registered.",
                self.tool_name
            )
        } else {
            write!(
                f,
                "unknown tool: '{}'. Available: {}",
                self.tool_name,
                self.available.join(", ")
            )
        }
    }
}

impl std::error::Error for UnknownToolError {}

/// Result of executing a tool call.
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    /// True if the error is due to malformed arguments (JSON parse or validation failure)
    /// and the model should be given a chance to retry with corrected arguments.
    pub is_retryable: bool,
}

impl ToolExecutionResult {
    /// Returns true if the execution succeeded.
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }

    /// Returns true if the execution failed.
    pub fn is_err(&self) -> bool {
        self.error.is_some()
    }
}

/// A boxed future type for async tool handlers.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Handler function type for tool execution.
/// Receives parsed JSON arguments and the original tool call.
/// Returns a JSON-serializable result or an error message.
pub type ToolHandler =
    Arc<dyn Fn(Value, ToolCall) -> BoxFuture<'static, Result<Value, String>> + Send + Sync>;

/// Registry for mapping tool names to handler functions with automatic dispatch.
///
/// # Example
///
/// ```ignore
/// use modelrelay::{ToolRegistry, tool_handler};
/// use serde_json::json;
///
/// let registry = ToolRegistry::new()
///     .register("get_weather", tool_handler!(|args, _call| async move {
///         let location = args.get("location").and_then(|v| v.as_str()).unwrap_or("unknown");
///         Ok(json!({ "temp": 72, "unit": "fahrenheit", "location": location }))
///     }))
///     .register("search", tool_handler!(|args, _call| async move {
///         let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
///         Ok(json!({ "results": ["result1", "result2"], "query": query }))
///     }));
///
/// // Execute all tool calls from a response
/// let results = registry.execute_all(&response.tool_calls.unwrap_or_default()).await;
///
/// // Convert results to messages for the next request
/// let messages = registry.results_to_messages(&results);
/// ```
pub struct ToolRegistry {
    handlers: HashMap<String, ToolHandler>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    /// Creates a new empty tool registry.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Registers a handler function for a tool name.
    /// Returns self for method chaining.
    pub fn register(mut self, name: impl Into<String>, handler: ToolHandler) -> Self {
        self.handlers.insert(name.into(), handler);
        self
    }

    /// Registers a handler function for a tool name (mutable reference version).
    pub fn register_mut(&mut self, name: impl Into<String>, handler: ToolHandler) -> &mut Self {
        self.handlers.insert(name.into(), handler);
        self
    }

    /// Unregisters a tool handler.
    /// Returns true if the handler was removed, false if it didn't exist.
    pub fn unregister(&mut self, name: &str) -> bool {
        self.handlers.remove(name).is_some()
    }

    /// Checks if a handler is registered for the given tool name.
    pub fn has(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    /// Returns the list of registered tool names.
    pub fn registered_tools(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    /// Executes a single tool call.
    pub async fn execute(&self, call: &ToolCall) -> ToolExecutionResult {
        let tool_name = call
            .function
            .as_ref()
            .map(|f| f.name.clone())
            .unwrap_or_default();

        let handler = match self.handlers.get(&tool_name) {
            Some(h) => h,
            None => {
                let error_msg = UnknownToolError {
                    tool_name: tool_name.clone(),
                    available: self.registered_tools(),
                }
                .to_string();
                return ToolExecutionResult {
                    tool_call_id: call.id.clone(),
                    tool_name,
                    result: None,
                    error: Some(error_msg),
                    is_retryable: false, // Unknown tool is not retryable
                };
            }
        };

        // Parse arguments
        let args: Value = match call.function.as_ref() {
            Some(f) if f.arguments.is_empty() => Value::Object(Default::default()),
            Some(f) => match serde_json::from_str(&f.arguments) {
                Ok(v) => v,
                Err(e) => {
                    return ToolExecutionResult {
                        tool_call_id: call.id.clone(),
                        tool_name,
                        result: None,
                        error: Some(format!("failed to parse tool arguments: {}", e)),
                        is_retryable: true, // JSON parse errors are retryable
                    };
                }
            },
            None => Value::Object(Default::default()),
        };

        // Execute handler
        match handler(args, call.clone()).await {
            Ok(result) => ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name,
                result: Some(result),
                error: None,
                is_retryable: false,
            },
            Err(e) => {
                // Check if error message indicates a validation/parse error (retryable)
                let is_retryable = e.starts_with("invalid arguments")
                    || e.starts_with("failed to parse")
                    || e.contains("validation");
                ToolExecutionResult {
                    tool_call_id: call.id.clone(),
                    tool_name,
                    result: None,
                    error: Some(e),
                    is_retryable,
                }
            }
        }
    }

    /// Executes multiple tool calls concurrently.
    /// Results are returned in the same order as the input calls.
    pub async fn execute_all(&self, calls: &[ToolCall]) -> Vec<ToolExecutionResult> {
        let futures: Vec<_> = calls.iter().map(|call| self.execute(call)).collect();
        futures::future::join_all(futures).await
    }

    /// Converts execution results to tool result messages.
    /// Useful for appending to the conversation history.
    pub fn results_to_messages(&self, results: &[ToolExecutionResult]) -> Vec<InputItem> {
        results
            .iter()
            .map(|r| {
                let content = if let Some(ref error) = r.error {
                    format!("Error: {}", error)
                } else if let Some(ref result) = r.result {
                    match result {
                        Value::String(s) => s.clone(),
                        _ => serde_json::to_string(result).unwrap_or_default(),
                    }
                } else {
                    String::new()
                };
                tool_result_message(&r.tool_call_id, content)
            })
            .collect()
    }
}

/// Helper macro to create a tool handler from an async closure.
///
/// # Example
///
/// ```ignore
/// use modelrelay::tool_handler;
/// use serde_json::json;
///
/// let handler = tool_handler!(|args, call| async move {
///     let location = args.get("location").and_then(|v| v.as_str()).unwrap_or("unknown");
///     Ok(json!({ "temp": 72, "location": location }))
/// });
/// ```
#[macro_export]
macro_rules! tool_handler {
    ($closure:expr) => {{
        use std::sync::Arc;
        let handler: $crate::tools::ToolHandler =
            Arc::new(move |args, call| Box::pin($closure(args, call)));
        handler
    }};
}

/// Creates a tool handler from a synchronous function.
/// The function receives parsed JSON arguments and returns a JSON-serializable result.
pub fn sync_handler<F>(f: F) -> ToolHandler
where
    F: Fn(Value, ToolCall) -> Result<Value, String> + Send + Sync + 'static,
{
    Arc::new(move |args, call| {
        let result = f(args, call);
        Box::pin(async move { result })
    })
}

// ============================================================================
// Schema Inference (requires "schema" feature)
// ============================================================================

/// Creates a function tool from a type that implements `schemars::JsonSchema`.
///
/// This function automatically generates a JSON Schema from the Rust type
/// using the `schemars` crate, eliminating the need to manually write JSON schemas.
///
/// # Example
///
/// ```ignore
/// use schemars::JsonSchema;
/// use serde::Deserialize;
/// use modelrelay::function_tool_from_type;
///
/// #[derive(JsonSchema, Deserialize)]
/// struct GetWeatherParams {
///     /// City name
///     location: String,
///     /// Temperature unit
///     #[serde(default = "default_unit")]
///     unit: Option<String>,
/// }
///
/// fn default_unit() -> String { "celsius".to_string() }
///
/// let tool = function_tool_from_type::<GetWeatherParams>("get_weather", "Get weather for a location");
/// ```
pub fn function_tool_from_type<T: schemars::JsonSchema>(
    name: impl Into<String>,
    description: impl Into<String>,
) -> Tool {
    let schema = schemars::schema_for!(T);
    let parameters = serde_json::to_value(&schema).ok();

    Tool {
        kind: ToolType::Function,
        function: Some(FunctionTool {
            name: name.into(),
            description: Some(description.into()),
            parameters,
        }),
        x_search: None,
        code_execution: None,
    }
}

/// Typed tool definition + argument parser bound to a Rust type.
pub struct TypedTool<T> {
    name: String,
    description: String,
    _marker: PhantomData<T>,
}

/// A tool call with parsed arguments.
#[derive(Debug)]
pub struct TypedToolCall<T> {
    pub call: ToolCall,
    pub args: T,
}

impl<T> TypedTool<T>
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned,
{
    /// Create a typed tool wrapper from a name + description.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            _marker: PhantomData,
        }
    }

    /// Returns the tool definition for API requests.
    pub fn definition(&self) -> Tool {
        function_tool_from_type::<T>(&self.name, &self.description)
    }

    /// Returns the tool name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Parses a tool call, ensuring the tool name matches.
    pub fn parse_call(&self, call: &ToolCall) -> Result<TypedToolCall<T>, ToolArgsError> {
        let func = call.function.as_ref().ok_or_else(|| ToolArgsError {
            message: "tool call missing function".to_string(),
            tool_call_id: call.id.clone(),
            tool_name: self.name.clone(),
            raw_arguments: "".to_string(),
        })?;

        if func.name != self.name {
            return Err(ToolArgsError {
                message: format!("expected tool '{}', got '{}'", self.name, func.name),
                tool_call_id: call.id.clone(),
                tool_name: self.name.clone(),
                raw_arguments: func.arguments.clone(),
            });
        }

        let args: T = parse_tool_args(call)?;
        Ok(TypedToolCall {
            call: call.clone(),
            args,
        })
    }
}

/// Trait extension for types that implement `JsonSchema` to easily convert to a Tool.
///
/// # Example
///
/// ```ignore
/// use schemars::JsonSchema;
/// use serde::Deserialize;
/// use modelrelay::ToolSchema;
///
/// #[derive(JsonSchema, Deserialize)]
/// struct SearchParams {
///     /// The search query
///     query: String,
///     /// Maximum number of results
///     max_results: Option<i32>,
/// }
///
/// let tool = SearchParams::as_tool("search", "Search for information");
/// ```
pub trait ToolSchema: schemars::JsonSchema + Sized {
    /// Creates a function tool from this type's JSON Schema.
    fn as_tool(name: impl Into<String>, description: impl Into<String>) -> Tool {
        function_tool_from_type::<Self>(name, description)
    }
}

impl<T: schemars::JsonSchema + Sized> ToolSchema for T {}

// ============================================================================
// ToolBuilder - Fluent builder for tool definitions + handlers
// ============================================================================

/// Fluent builder for defining tools with both schemas and handlers.
///
/// `ToolBuilder` combines tool definitions (sent to the API) with tool handlers
/// (for local execution), similar to the TypeScript/Go SDK patterns.
///
/// # Example
///
/// ```ignore
/// use modelrelay::{ToolBuilder, ToolCall};
/// use schemars::JsonSchema;
/// use serde::Deserialize;
/// use serde_json::json;
///
/// #[derive(JsonSchema, Deserialize)]
/// struct WeatherArgs {
///     /// City name
///     location: String,
///     /// Temperature unit (optional)
///     unit: Option<String>,
/// }
///
/// let tools = ToolBuilder::new()
///     .add_sync::<WeatherArgs, _>("get_weather", "Get weather for a location", |args, _call| {
///         Ok(json!({ "temp": 72, "location": args.location }))
///     })
///     .add_async::<SearchArgs, _>("search", "Search the web", |args, _call| {
///         Box::pin(async move {
///             Ok(json!({ "results": ["result1", "result2"] }))
///         })
///     });
///
/// // Extract definitions for API request
/// let (definitions, registry) = tools.build();
/// ```
pub struct ToolBuilder {
    definitions: Vec<Tool>,
    registry: ToolRegistry,
}

impl Default for ToolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolBuilder {
    /// Creates a new empty tool builder.
    pub fn new() -> Self {
        Self {
            definitions: Vec::new(),
            registry: ToolRegistry::new(),
        }
    }

    /// Adds a synchronous tool handler with type-safe arguments.
    ///
    /// The argument type must implement `JsonSchema` (for generating the schema)
    /// and `DeserializeOwned` (for parsing incoming arguments).
    ///
    /// # Example
    ///
    /// ```ignore
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    /// use modelrelay::ToolBuilder;
    ///
    /// #[derive(JsonSchema, Deserialize)]
    /// struct Args { path: String }
    ///
    /// let tools = ToolBuilder::new()
    ///     .add_sync::<Args, _>("read_file", "Read a file", |args, _call| {
    ///         Ok(serde_json::json!({ "content": "file contents" }))
    ///     });
    /// ```
    pub fn add_sync<T, F>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        T: schemars::JsonSchema + serde::de::DeserializeOwned + Send + 'static,
        F: Fn(T, ToolCall) -> Result<Value, String> + Send + Sync + 'static,
    {
        let name_str = name.into();
        let desc_str = description.into();

        // Add tool definition using schema from type
        let tool = function_tool_from_type::<T>(&name_str, &desc_str);
        self.definitions.push(tool);

        // Register handler that parses args and calls the typed handler
        let handler: ToolHandler = Arc::new(move |args: Value, call: ToolCall| {
            let parsed: Result<T, _> = serde_json::from_value(args);
            match parsed {
                Ok(typed_args) => {
                    let result = handler(typed_args, call);
                    Box::pin(async move { result })
                }
                Err(e) => Box::pin(async move { Err(format!("failed to parse arguments: {}", e)) }),
            }
        });
        self.registry.register_mut(&name_str, handler);

        self
    }

    /// Adds an asynchronous tool handler with type-safe arguments.
    ///
    /// The handler returns a boxed future for async execution.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    /// use modelrelay::ToolBuilder;
    ///
    /// #[derive(JsonSchema, Deserialize)]
    /// struct Args { query: String }
    ///
    /// let tools = ToolBuilder::new()
    ///     .add_async::<Args>("search", "Search the web", |args, _call| {
    ///         Box::pin(async move {
    ///             Ok(serde_json::json!({ "results": ["result"] }))
    ///         })
    ///     });
    /// ```
    pub fn add_async<T, F>(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        T: schemars::JsonSchema + serde::de::DeserializeOwned + Send + 'static,
        F: Fn(T, ToolCall) -> BoxFuture<'static, Result<Value, String>> + Send + Sync + 'static,
    {
        let name_str = name.into();
        let desc_str = description.into();

        // Add tool definition using schema from type
        let tool = function_tool_from_type::<T>(&name_str, &desc_str);
        self.definitions.push(tool);

        // Register handler that parses args and calls the typed async handler
        let handler = Arc::new(handler);
        let async_handler: ToolHandler = Arc::new(move |args: Value, call: ToolCall| {
            let parsed: Result<T, _> = serde_json::from_value(args);
            let handler = handler.clone();
            match parsed {
                Ok(typed_args) => handler(typed_args, call),
                Err(e) => Box::pin(async move { Err(format!("failed to parse arguments: {}", e)) }),
            }
        });
        self.registry.register_mut(&name_str, async_handler);

        self
    }

    /// Adds a raw tool handler without type-safe argument parsing.
    ///
    /// Use this when you want to handle argument parsing yourself or need
    /// more control over the handler behavior.
    pub fn add_raw(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Option<Value>,
        handler: ToolHandler,
    ) -> Self {
        let name_str = name.into();
        let desc_str = description.into();

        let tool = Tool {
            kind: ToolType::Function,
            function: Some(FunctionTool {
                name: name_str.clone(),
                description: Some(desc_str),
                parameters,
            }),
            x_search: None,
            code_execution: None,
        };
        self.definitions.push(tool);
        self.registry.register_mut(&name_str, handler);

        self
    }

    /// Returns just the tool definitions (for API requests).
    pub fn definitions(&self) -> Vec<Tool> {
        self.definitions.clone()
    }

    /// Returns just the tool registry (for execution).
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Consumes the builder and returns both definitions and registry.
    ///
    /// This is typically called when passing tools to an agent loop.
    pub fn build(self) -> (Vec<Tool>, ToolRegistry) {
        (self.definitions, self.registry)
    }
}

// ============================================================================
// Retry Utilities
// ============================================================================

/// Formats a tool execution error for sending back to the model.
///
/// If the error is retryable (e.g., JSON parse or validation error), includes
/// a message asking the model to correct the arguments and try again.
pub fn format_tool_error_for_model(result: &ToolExecutionResult) -> String {
    let error_msg = result.error.as_deref().unwrap_or("unknown error");
    let mut lines = format!("Tool call error for '{}': {}", result.tool_name, error_msg);
    if result.is_retryable {
        lines.push_str("\n\nPlease correct the arguments and try again.");
    }
    lines
}

/// Returns true if any of the results have a retryable error.
pub fn has_retryable_errors(results: &[ToolExecutionResult]) -> bool {
    results.iter().any(|r| r.error.is_some() && r.is_retryable)
}

/// Filters the results to only those with retryable errors.
pub fn get_retryable_errors(results: &[ToolExecutionResult]) -> Vec<&ToolExecutionResult> {
    results
        .iter()
        .filter(|r| r.error.is_some() && r.is_retryable)
        .collect()
}

/// Creates tool result messages for retryable errors, suitable for sending
/// back to the model to prompt a retry.
pub fn create_retry_messages(results: &[ToolExecutionResult]) -> Vec<InputItem> {
    results
        .iter()
        .filter(|r| r.error.is_some() && r.is_retryable)
        .map(|r| tool_result_message(&r.tool_call_id, format_tool_error_for_model(r)))
        .collect()
}

/// Options for the `execute_with_retry` function.
pub struct RetryOptions<F>
where
    F: Fn(Vec<InputItem>, usize) -> BoxFuture<'static, Result<Vec<ToolCall>, String>>,
{
    /// Maximum number of retry attempts (default: 2).
    pub max_retries: usize,
    /// Callback to get new tool calls from the model after a retry.
    /// Receives the error messages and the attempt number (1-indexed).
    /// Should return new tool calls or an error.
    pub on_retry: F,
}

/// Executes tool calls with automatic retry for malformed arguments.
///
/// When tool calls fail due to JSON parse or validation errors, this function
/// will use the provided callback to get corrected tool calls from the model
/// and retry execution.
///
/// # Result Preservation
///
/// Successful results are preserved across retries. If you execute multiple tool
/// calls and only some fail, the successful results are kept and merged with the
/// results from retry attempts. Results are keyed by `tool_call_id`, so if a retry
/// returns a call with the same ID as a previous result, the newer result will
/// replace it.
///
/// # Arguments
///
/// * `registry` - The tool registry to use for execution
/// * `tool_calls` - Initial tool calls to execute
/// * `options` - Retry options including max retries and the retry callback
///
/// # Returns
///
/// Returns all execution results after retries complete, including preserved
/// successes from earlier attempts.
///
/// # Example
///
/// ```ignore
/// use modelrelay::{execute_with_retry, RetryOptions, ToolRegistry};
///
/// let results = execute_with_retry(
///     &registry,
///     tool_calls,
///     RetryOptions {
///         max_retries: 2,
///         on_retry: |error_messages, attempt| Box::pin(async move {
///             // Send error_messages back to model and get new tool calls
///             // This is where you'd call the LLM API
///             Ok(new_tool_calls)
///         }),
///     },
/// ).await;
/// ```
pub async fn execute_with_retry<F>(
    registry: &ToolRegistry,
    tool_calls: Vec<ToolCall>,
    options: RetryOptions<F>,
) -> Vec<ToolExecutionResult>
where
    F: Fn(Vec<InputItem>, usize) -> BoxFuture<'static, Result<Vec<ToolCall>, String>>,
{
    let mut current_calls = tool_calls;
    let mut attempt = 0;

    // Track successful results across retries, keyed by tool_call_id
    let mut successful_results: HashMap<String, ToolExecutionResult> = HashMap::new();

    loop {
        let results = registry.execute_all(&current_calls).await;

        // Store successful results (non-error or non-retryable error)
        for result in &results {
            if result.error.is_none() || !result.is_retryable {
                successful_results.insert(result.tool_call_id.clone(), result.clone());
            }
        }

        // If no retryable errors or we've exhausted retries, return all results
        if !has_retryable_errors(&results) || attempt >= options.max_retries {
            // Include any remaining retryable errors in the final results
            for result in results {
                if result.error.is_some() && result.is_retryable {
                    successful_results.insert(result.tool_call_id.clone(), result);
                }
            }
            return successful_results.into_values().collect();
        }

        // Create error messages for the model
        let error_messages = create_retry_messages(&results);

        // Get the retryable results for potential inclusion if callback fails
        let retryable: Vec<_> = results
            .into_iter()
            .filter(|r| r.error.is_some() && r.is_retryable)
            .collect();

        // Get new tool calls from the callback
        attempt += 1;
        match (options.on_retry)(error_messages, attempt).await {
            Ok(new_calls) if !new_calls.is_empty() => {
                current_calls = new_calls;
            }
            Ok(_) => {
                // Empty tool calls returned, stop retrying - include final failed results
                for result in retryable {
                    successful_results.insert(result.tool_call_id.clone(), result);
                }
                return successful_results.into_values().collect();
            }
            Err(_) => {
                // Callback error, stop retrying - include final failed results
                for result in retryable {
                    successful_results.insert(result.tool_call_id.clone(), result);
                }
                return successful_results.into_values().collect();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FunctionCallDelta;

    #[test]
    fn test_function_tool_creation() {
        let tool = Tool::function("get_weather", Some("Get the weather".into()), None);
        assert_eq!(tool.kind, ToolType::Function);
        assert_eq!(tool.function.as_ref().unwrap().name, "get_weather");
    }

    #[test]
    fn test_tool_result_message() {
        let msg = tool_result_message("call_123", "sunny");
        match msg {
            InputItem::Message {
                role,
                content,
                tool_call_id,
                ..
            } => {
                assert_eq!(role, crate::types::MessageRole::Tool);
                let text = content
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text { text } => Some(text.as_str()),
                    })
                    .collect::<String>();
                assert_eq!(text, "sunny");
                assert_eq!(tool_call_id.as_deref(), Some("call_123"));
            }
        }
    }

    #[test]
    fn test_tool_call_accumulator() {
        let mut acc = ToolCallAccumulator::new();

        // First delta starts a new tool call
        let delta1 = ToolCallDelta {
            index: 0,
            id: Some("call_1".to_string()),
            type_: Some("function".to_string()),
            function: Some(FunctionCallDelta {
                name: Some("get_weather".to_string()),
                arguments: Some("{\"loc".to_string()),
            }),
        };
        assert!(acc.process_delta(&delta1));

        // Second delta appends
        let delta2 = ToolCallDelta {
            index: 0,
            id: None,
            type_: None,
            function: Some(FunctionCallDelta {
                name: None,
                arguments: Some("ation\":\"NYC\"}".to_string()),
            }),
        };
        assert!(!acc.process_delta(&delta2));

        let calls = acc.get_tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(
            calls[0].function.as_ref().unwrap().arguments,
            "{\"location\":\"NYC\"}"
        );
    }

    #[test]
    fn test_tool_registry_has_and_registered_tools() {
        let registry = ToolRegistry::new()
            .register("tool_a", sync_handler(|_, _| Ok(Value::Null)))
            .register("tool_b", sync_handler(|_, _| Ok(Value::Null)));

        assert!(registry.has("tool_a"));
        assert!(registry.has("tool_b"));
        assert!(!registry.has("tool_c"));

        let tools = registry.registered_tools();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"tool_a".to_string()));
        assert!(tools.contains(&"tool_b".to_string()));
    }

    #[test]
    fn test_tool_registry_unregister() {
        let mut registry = ToolRegistry::new();
        registry.register_mut("tool_a", sync_handler(|_, _| Ok(Value::Null)));

        assert!(registry.has("tool_a"));
        assert!(registry.unregister("tool_a"));
        assert!(!registry.has("tool_a"));
        assert!(!registry.unregister("tool_a")); // Already removed
    }

    #[tokio::test]
    async fn test_tool_registry_execute_success() {
        let registry = ToolRegistry::new().register(
            "get_weather",
            sync_handler(|args, _call| {
                let location = args
                    .get("location")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                Ok(serde_json::json!({ "temp": 72, "location": location }))
            }),
        );

        let call = ToolCall {
            id: "call_123".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"location":"NYC"}"#.to_string(),
            }),
        };

        let result = registry.execute(&call).await;
        assert!(result.is_ok());
        assert_eq!(result.tool_call_id, "call_123");
        assert_eq!(result.tool_name, "get_weather");

        let value = result.result.unwrap();
        assert_eq!(value.get("temp").unwrap(), 72);
        assert_eq!(value.get("location").unwrap(), "NYC");
    }

    #[tokio::test]
    async fn test_tool_registry_execute_unknown_tool() {
        let registry =
            ToolRegistry::new().register("known_tool", sync_handler(|_, _| Ok(Value::Null)));

        let call = ToolCall {
            id: "call_456".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "unknown_tool".to_string(),
                arguments: "{}".to_string(),
            }),
        };

        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result.error.as_ref().unwrap().contains("unknown tool"));
        assert!(result.error.as_ref().unwrap().contains("unknown_tool"));
        assert!(result.error.as_ref().unwrap().contains("known_tool"));
    }

    #[tokio::test]
    async fn test_tool_registry_execute_handler_error() {
        let registry = ToolRegistry::new().register(
            "failing_tool",
            sync_handler(|_, _| Err("something went wrong".to_string())),
        );

        let call = ToolCall {
            id: "call_789".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "failing_tool".to_string(),
                arguments: "{}".to_string(),
            }),
        };

        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert_eq!(result.error.as_ref().unwrap(), "something went wrong");
    }

    #[tokio::test]
    async fn test_tool_registry_execute_malformed_json() {
        let registry =
            ToolRegistry::new().register("my_tool", sync_handler(|_, _| Ok(Value::Null)));

        let call = ToolCall {
            id: "call_bad".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "my_tool".to_string(),
                arguments: "{not valid json".to_string(),
            }),
        };

        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result
            .error
            .as_ref()
            .unwrap()
            .contains("failed to parse tool arguments"));
    }

    #[tokio::test]
    async fn test_tool_registry_execute_all() {
        let registry = ToolRegistry::new()
            .register(
                "tool_a",
                sync_handler(|_, _| Ok(serde_json::json!("result_a"))),
            )
            .register(
                "tool_b",
                sync_handler(|_, _| Ok(serde_json::json!("result_b"))),
            );

        let calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                kind: ToolType::Function,
                function: Some(FunctionCall {
                    name: "tool_a".to_string(),
                    arguments: "{}".to_string(),
                }),
            },
            ToolCall {
                id: "call_2".to_string(),
                kind: ToolType::Function,
                function: Some(FunctionCall {
                    name: "tool_b".to_string(),
                    arguments: "{}".to_string(),
                }),
            },
        ];

        let results = registry.execute_all(&calls).await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].tool_call_id, "call_1");
        assert_eq!(results[1].tool_call_id, "call_2");
        assert!(results[0].is_ok());
        assert!(results[1].is_ok());
    }

    #[tokio::test]
    async fn test_tool_registry_results_to_messages() {
        let registry = ToolRegistry::new()
            .register(
                "success_tool",
                sync_handler(|_, _| Ok(serde_json::json!({"data": "success"}))),
            )
            .register("error_tool", sync_handler(|_, _| Err("failed".to_string())));

        let calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                kind: ToolType::Function,
                function: Some(FunctionCall {
                    name: "success_tool".to_string(),
                    arguments: "{}".to_string(),
                }),
            },
            ToolCall {
                id: "call_2".to_string(),
                kind: ToolType::Function,
                function: Some(FunctionCall {
                    name: "error_tool".to_string(),
                    arguments: "{}".to_string(),
                }),
            },
        ];

        let results = registry.execute_all(&calls).await;
        let messages = registry.results_to_messages(&results);

        assert_eq!(messages.len(), 2);

        for (idx, expected_call_id) in [("call_1", "success"), ("call_2", "Error:")]
            .into_iter()
            .enumerate()
        {
            let msg = &messages[idx];
            match msg {
                InputItem::Message {
                    role,
                    content,
                    tool_call_id,
                    ..
                } => {
                    assert_eq!(*role, crate::types::MessageRole::Tool);
                    assert_eq!(tool_call_id.as_deref(), Some(expected_call_id.0));
                    let text = content
                        .iter()
                        .filter_map(|p| match p {
                            ContentPart::Text { text } => Some(text.as_str()),
                        })
                        .collect::<String>();
                    assert!(
                        text.contains(expected_call_id.1) || text.starts_with(expected_call_id.1),
                        "unexpected content: {}",
                        text
                    );
                }
            }
        }
    }

    #[test]
    fn test_unknown_tool_error_display() {
        let err = UnknownToolError {
            tool_name: "foo".to_string(),
            available: vec!["bar".to_string(), "baz".to_string()],
        };
        assert_eq!(err.to_string(), "unknown tool: 'foo'. Available: bar, baz");

        let err_empty = UnknownToolError {
            tool_name: "foo".to_string(),
            available: vec![],
        };
        assert_eq!(
            err_empty.to_string(),
            "unknown tool: 'foo'. No tools registered."
        );
    }

    // ========================================
    // Type-safe Argument Parsing Tests
    // ========================================

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct WeatherArgs {
        location: String,
        #[serde(default)]
        unit: Option<String>,
    }

    #[test]
    fn test_parse_tool_args_success() {
        let call = ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"location":"NYC","unit":"celsius"}"#.to_string(),
            }),
        };

        let args: WeatherArgs = parse_tool_args(&call).unwrap();
        assert_eq!(args.location, "NYC");
        assert_eq!(args.unit, Some("celsius".to_string()));
    }

    #[test]
    fn test_parse_tool_args_with_defaults() {
        let call = ToolCall {
            id: "call_2".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"location":"London"}"#.to_string(),
            }),
        };

        let args: WeatherArgs = parse_tool_args(&call).unwrap();
        assert_eq!(args.location, "London");
        assert_eq!(args.unit, None); // Uses default
    }

    #[test]
    fn test_parse_tool_args_empty_arguments() {
        let call = ToolCall {
            id: "call_3".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "list_items".to_string(),
                arguments: "".to_string(),
            }),
        };

        // Should parse as empty object
        let args: std::collections::HashMap<String, String> = parse_tool_args(&call).unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_tool_args_invalid_json() {
        let call = ToolCall {
            id: "call_4".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "get_weather".to_string(),
                arguments: "{not valid json".to_string(),
            }),
        };

        let err = parse_tool_args::<WeatherArgs>(&call).unwrap_err();
        assert!(err.message.contains("failed to parse arguments"));
        assert!(err.message.contains("get_weather"));
        assert_eq!(err.tool_call_id, "call_4");
        assert_eq!(err.tool_name, "get_weather");
    }

    #[test]
    fn test_parse_tool_args_missing_required_field() {
        let call = ToolCall {
            id: "call_5".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"unit":"celsius"}"#.to_string(), // Missing location
            }),
        };

        let err = parse_tool_args::<WeatherArgs>(&call).unwrap_err();
        assert!(err.message.contains("failed to parse arguments"));
    }

    #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
    struct ReadFileArgs {
        path: String,
    }

    #[test]
    fn test_typed_tool_parse_call_success() {
        let tool = TypedTool::<ReadFileArgs>::new("read_file", "Read a file");
        let call = ToolCall {
            id: "call_typed_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "read_file".to_string(),
                arguments: r#"{"path":"/tmp/config.json"}"#.to_string(),
            }),
        };

        let typed = tool.parse_call(&call).unwrap();
        assert_eq!(typed.args.path, "/tmp/config.json");
        assert_eq!(typed.call.id, "call_typed_1");
    }

    #[test]
    fn test_typed_tool_parse_call_name_mismatch() {
        let tool = TypedTool::<ReadFileArgs>::new("read_file", "Read a file");
        let call = ToolCall {
            id: "call_typed_2".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "other_tool".to_string(),
                arguments: r#"{"path":"/tmp/config.json"}"#.to_string(),
            }),
        };

        let err = tool.parse_call(&call).unwrap_err();
        assert!(err.message.contains("expected tool 'read_file'"));
    }

    #[test]
    fn test_typed_tool_parse_call_missing_function() {
        let tool = TypedTool::<ReadFileArgs>::new("read_file", "Read a file");
        let call = ToolCall {
            id: "call_typed_3".to_string(),
            kind: ToolType::Function,
            function: None,
        };

        let err = tool.parse_call(&call).unwrap_err();
        assert!(err.message.contains("tool call missing function"));
    }

    #[derive(Debug, serde::Deserialize)]
    struct ValidatedArgs {
        value: i32,
    }

    impl ValidateArgs for ValidatedArgs {
        fn validate(&self) -> Result<(), String> {
            if self.value < 0 {
                return Err("value must be non-negative".to_string());
            }
            if self.value > 100 {
                return Err("value must be at most 100".to_string());
            }
            Ok(())
        }
    }

    #[test]
    fn test_parse_and_validate_tool_args_success() {
        let call = ToolCall {
            id: "call_6".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "set_value".to_string(),
                arguments: r#"{"value":50}"#.to_string(),
            }),
        };

        let args: ValidatedArgs = parse_and_validate_tool_args(&call).unwrap();
        assert_eq!(args.value, 50);
    }

    #[test]
    fn test_parse_and_validate_tool_args_validation_failure() {
        let call = ToolCall {
            id: "call_7".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "set_value".to_string(),
                arguments: r#"{"value":-5}"#.to_string(),
            }),
        };

        let err = parse_and_validate_tool_args::<ValidatedArgs>(&call).unwrap_err();
        assert!(err.message.contains("invalid arguments"));
        assert!(err.message.contains("value must be non-negative"));
    }

    #[test]
    fn test_tool_args_error_display() {
        let err = ToolArgsError {
            message: "test error message".to_string(),
            tool_call_id: "call_123".to_string(),
            tool_name: "my_tool".to_string(),
            raw_arguments: "{}".to_string(),
        };
        assert_eq!(err.to_string(), "test error message");
    }

    // ========================================
    // Retry Utilities Tests
    // ========================================

    #[test]
    fn test_format_tool_error_for_model_retryable() {
        let result = ToolExecutionResult {
            tool_call_id: "call_1".to_string(),
            tool_name: "my_tool".to_string(),
            result: None,
            error: Some("failed to parse arguments".to_string()),
            is_retryable: true,
        };

        let formatted = format_tool_error_for_model(&result);
        assert!(formatted.contains("Tool call error for 'my_tool'"));
        assert!(formatted.contains("failed to parse arguments"));
        assert!(formatted.contains("Please correct the arguments and try again"));
    }

    #[test]
    fn test_format_tool_error_for_model_not_retryable() {
        let result = ToolExecutionResult {
            tool_call_id: "call_1".to_string(),
            tool_name: "my_tool".to_string(),
            result: None,
            error: Some("internal error".to_string()),
            is_retryable: false,
        };

        let formatted = format_tool_error_for_model(&result);
        assert!(formatted.contains("Tool call error for 'my_tool'"));
        assert!(formatted.contains("internal error"));
        assert!(!formatted.contains("Please correct the arguments"));
    }

    #[test]
    fn test_has_retryable_errors() {
        let results = vec![
            ToolExecutionResult {
                tool_call_id: "call_1".to_string(),
                tool_name: "tool_a".to_string(),
                result: Some(Value::String("ok".to_string())),
                error: None,
                is_retryable: false,
            },
            ToolExecutionResult {
                tool_call_id: "call_2".to_string(),
                tool_name: "tool_b".to_string(),
                result: None,
                error: Some("parse error".to_string()),
                is_retryable: true,
            },
        ];

        assert!(has_retryable_errors(&results));

        // No retryable errors
        let results_no_retry = vec![ToolExecutionResult {
            tool_call_id: "call_1".to_string(),
            tool_name: "tool_a".to_string(),
            result: Some(Value::String("ok".to_string())),
            error: None,
            is_retryable: false,
        }];
        assert!(!has_retryable_errors(&results_no_retry));

        // Error but not retryable
        let results_not_retryable = vec![ToolExecutionResult {
            tool_call_id: "call_1".to_string(),
            tool_name: "tool_a".to_string(),
            result: None,
            error: Some("internal error".to_string()),
            is_retryable: false,
        }];
        assert!(!has_retryable_errors(&results_not_retryable));
    }

    #[test]
    fn test_get_retryable_errors() {
        let results = vec![
            ToolExecutionResult {
                tool_call_id: "call_1".to_string(),
                tool_name: "tool_a".to_string(),
                result: Some(Value::String("ok".to_string())),
                error: None,
                is_retryable: false,
            },
            ToolExecutionResult {
                tool_call_id: "call_2".to_string(),
                tool_name: "tool_b".to_string(),
                result: None,
                error: Some("parse error".to_string()),
                is_retryable: true,
            },
            ToolExecutionResult {
                tool_call_id: "call_3".to_string(),
                tool_name: "tool_c".to_string(),
                result: None,
                error: Some("validation error".to_string()),
                is_retryable: true,
            },
        ];

        let retryable = get_retryable_errors(&results);
        assert_eq!(retryable.len(), 2);
        assert_eq!(retryable[0].tool_call_id, "call_2");
        assert_eq!(retryable[1].tool_call_id, "call_3");
    }

    #[test]
    fn test_create_retry_messages() {
        let results = vec![
            ToolExecutionResult {
                tool_call_id: "call_1".to_string(),
                tool_name: "tool_a".to_string(),
                result: Some(Value::String("ok".to_string())),
                error: None,
                is_retryable: false,
            },
            ToolExecutionResult {
                tool_call_id: "call_2".to_string(),
                tool_name: "tool_b".to_string(),
                result: None,
                error: Some("parse error".to_string()),
                is_retryable: true,
            },
        ];

        let messages = create_retry_messages(&results);
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            InputItem::Message {
                role,
                content,
                tool_call_id,
                ..
            } => {
                assert_eq!(*role, crate::types::MessageRole::Tool);
                assert_eq!(tool_call_id.as_deref(), Some("call_2"));
                let text = content
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text { text } => Some(text.as_str()),
                    })
                    .collect::<String>();
                assert!(text.contains("Tool call error"));
                assert!(text.contains("Please correct the arguments"));
            }
        }
    }

    #[tokio::test]
    async fn test_execute_sets_is_retryable_for_json_parse_error() {
        let registry =
            ToolRegistry::new().register("my_tool", sync_handler(|_, _| Ok(Value::Null)));

        let call = ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "my_tool".to_string(),
                arguments: "{invalid json".to_string(),
            }),
        };

        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result.is_retryable);
        assert!(result.error.as_ref().unwrap().contains("failed to parse"));
    }

    #[tokio::test]
    async fn test_execute_sets_is_retryable_for_validation_error() {
        let registry = ToolRegistry::new().register(
            "my_tool",
            sync_handler(|_, _| Err("invalid arguments: missing field".to_string())),
        );

        let call = ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "my_tool".to_string(),
                arguments: "{}".to_string(),
            }),
        };

        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(result.is_retryable);
    }

    #[tokio::test]
    async fn test_execute_not_retryable_for_other_errors() {
        let registry = ToolRegistry::new().register(
            "my_tool",
            sync_handler(|_, _| Err("network timeout".to_string())),
        );

        let call = ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "my_tool".to_string(),
                arguments: "{}".to_string(),
            }),
        };

        let result = registry.execute(&call).await;
        assert!(result.is_err());
        assert!(!result.is_retryable);
    }

    #[tokio::test]
    async fn test_execute_with_retry_no_errors() {
        let registry = ToolRegistry::new().register(
            "my_tool",
            sync_handler(|_, _| Ok(serde_json::json!("success"))),
        );

        let calls = vec![ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "my_tool".to_string(),
                arguments: "{}".to_string(),
            }),
        }];

        let results = execute_with_retry(
            &registry,
            calls,
            RetryOptions {
                max_retries: 2,
                on_retry: |_, _| Box::pin(async { Ok(vec![]) }),
            },
        )
        .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
    }

    #[tokio::test]
    async fn test_execute_with_retry_retries_on_parse_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let retry_count = Arc::new(AtomicUsize::new(0));
        let retry_count_clone = retry_count.clone();

        let registry = ToolRegistry::new().register(
            "my_tool",
            sync_handler(|_, _| Ok(serde_json::json!("success"))),
        );

        // First call with invalid JSON
        let initial_calls = vec![ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "my_tool".to_string(),
                arguments: "{invalid".to_string(),
            }),
        }];

        let results = execute_with_retry(
            &registry,
            initial_calls,
            RetryOptions {
                max_retries: 2,
                on_retry: move |_messages, _attempt| {
                    retry_count_clone.fetch_add(1, Ordering::SeqCst);
                    // Return corrected tool call
                    Box::pin(async {
                        Ok(vec![ToolCall {
                            id: "call_1_retry".to_string(),
                            kind: ToolType::Function,
                            function: Some(FunctionCall {
                                name: "my_tool".to_string(),
                                arguments: "{}".to_string(),
                            }),
                        }])
                    })
                },
            },
        )
        .await;

        // Should have retried and succeeded
        assert_eq!(retry_count.load(Ordering::SeqCst), 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
    }

    #[tokio::test]
    async fn test_execute_with_retry_respects_max_retries() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let retry_count = Arc::new(AtomicUsize::new(0));
        let retry_count_clone = retry_count.clone();

        let registry = ToolRegistry::new().register(
            "my_tool",
            sync_handler(|_, _| Ok(serde_json::json!("success"))),
        );

        // Always return invalid JSON
        let initial_calls = vec![ToolCall {
            id: "call_1".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: "my_tool".to_string(),
                arguments: "{invalid".to_string(),
            }),
        }];

        let results = execute_with_retry(
            &registry,
            initial_calls,
            RetryOptions {
                max_retries: 2,
                on_retry: move |_messages, _attempt| {
                    retry_count_clone.fetch_add(1, Ordering::SeqCst);
                    // Keep returning invalid JSON
                    Box::pin(async {
                        Ok(vec![ToolCall {
                            id: "call_retry".to_string(),
                            kind: ToolType::Function,
                            function: Some(FunctionCall {
                                name: "my_tool".to_string(),
                                arguments: "{still invalid".to_string(),
                            }),
                        }])
                    })
                },
            },
        )
        .await;

        // Should have retried exactly max_retries times
        assert_eq!(retry_count.load(Ordering::SeqCst), 2);
        // Last result should still be an error
        assert!(results[0].is_err());
    }

    #[tokio::test]
    async fn test_execute_with_retry_preserves_successful_results() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let retry_count = Arc::new(AtomicUsize::new(0));
        let retry_count_clone = retry_count.clone();

        // Register two tools: one that always succeeds, one for the retry
        let registry = ToolRegistry::new()
            .register(
                "success_tool",
                sync_handler(|_, _| Ok(serde_json::json!("success_result"))),
            )
            .register(
                "failing_tool",
                sync_handler(|_, _| Ok(serde_json::json!("fixed_result"))),
            );

        // Initial calls: one succeeds, one has invalid JSON
        let initial_calls = vec![
            ToolCall {
                id: "call_success".to_string(),
                kind: ToolType::Function,
                function: Some(FunctionCall {
                    name: "success_tool".to_string(),
                    arguments: "{}".to_string(),
                }),
            },
            ToolCall {
                id: "call_fail".to_string(),
                kind: ToolType::Function,
                function: Some(FunctionCall {
                    name: "failing_tool".to_string(),
                    arguments: "{invalid".to_string(),
                }),
            },
        ];

        let results = execute_with_retry(
            &registry,
            initial_calls,
            RetryOptions {
                max_retries: 2,
                on_retry: move |_messages, _attempt| {
                    retry_count_clone.fetch_add(1, Ordering::SeqCst);
                    // Return corrected tool call only for the failing one
                    Box::pin(async {
                        Ok(vec![ToolCall {
                            id: "call_fail_retry".to_string(),
                            kind: ToolType::Function,
                            function: Some(FunctionCall {
                                name: "failing_tool".to_string(),
                                arguments: "{}".to_string(),
                            }),
                        }])
                    })
                },
            },
        )
        .await;

        // Should have retried once
        assert_eq!(retry_count.load(Ordering::SeqCst), 1);

        // Should have 2 results: the original success and the retried success
        assert_eq!(results.len(), 2);

        // Find the original successful result
        let original_success = results
            .iter()
            .find(|r| r.tool_call_id == "call_success")
            .expect("original successful result was lost during retry");
        assert!(original_success.is_ok());
        assert_eq!(
            original_success.result.as_ref().unwrap(),
            &serde_json::json!("success_result")
        );

        // Find the retried result
        let retry_success = results
            .iter()
            .find(|r| r.tool_call_id == "call_fail_retry")
            .expect("retried result not found");
        assert!(retry_success.is_ok());
        assert_eq!(
            retry_success.result.as_ref().unwrap(),
            &serde_json::json!("fixed_result")
        );
    }

    #[test]
    fn test_parse_user_ask_args() {
        let call = ToolCall {
            id: "call_user_ask".to_string(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: USER_ASK_TOOL_NAME.to_string(),
                arguments: r#"{"question":"Pick one","options":[{"label":"A"}]}"#.to_string(),
            }),
        };

        let args = parse_user_ask_args(&call).expect("parse user.ask args");
        assert_eq!(args.question, "Pick one");
        assert_eq!(args.options.len(), 1);
        assert_eq!(args.options[0].label, "A");
    }

    #[test]
    fn test_serialize_user_ask_result() {
        let out = serialize_user_ask_result(&UserAskResponse {
            answer: "PostgreSQL".to_string(),
            is_freeform: false,
        })
        .expect("serialize user.ask result");
        assert!(out.contains("PostgreSQL"));
        assert!(out.contains("is_freeform"));
    }
}
