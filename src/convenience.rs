//! Convenience methods for common SDK operations.
//!
//! This module provides high-level ergonomic APIs for common use cases:
//! - [`Client::chat`] - Simple chat completion returning full Response
//! - [`Client::ask`] - Simple prompt returning just text
//! - [`Client::agent`] - Agentic tool loop to completion

use crate::client::Client;
use crate::errors::{Error, Result, TransportError, TransportErrorKind};
use crate::responses::{get_all_tool_calls_from_response, ResponseBuilder};
use crate::tools::{
    assistant_message_with_tool_calls, tool_result_message, ToolBuilder, ToolExecutionResult,
};
use crate::types::{Model, Response};

/// Options for [`Client::chat`] and [`Client::ask`].
#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    /// System prompt to prepend to the conversation.
    pub system: Option<String>,
    /// Customer ID for attributed requests (if set, model can be omitted).
    pub customer_id: Option<String>,
}

impl ChatOptions {
    /// Create new ChatOptions with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Set the customer ID for attributed requests.
    pub fn with_customer_id(mut self, customer_id: impl Into<String>) -> Self {
        self.customer_id = Some(customer_id.into());
        self
    }
}

/// Default maximum turns for agent loops.
pub const DEFAULT_MAX_TURNS: usize = 100;

/// Use this for max_turns to disable the turn limit.
/// Use with caution as this can lead to infinite loops and runaway API costs.
pub const NO_TURN_LIMIT: usize = usize::MAX;

/// Options for [`Client::agent`].
pub struct AgentOptions {
    /// Tools for the agent to use (includes both definitions and handlers).
    pub tools: ToolBuilder,
    /// The user's prompt.
    pub prompt: String,
    /// Optional system prompt.
    pub system: Option<String>,
    /// Maximum number of LLM calls.
    /// Default is 100. Set to NO_TURN_LIMIT for unlimited turns.
    pub max_turns: Option<usize>,
}

impl AgentOptions {
    /// Create new AgentOptions with required fields.
    pub fn new(tools: ToolBuilder, prompt: impl Into<String>) -> Self {
        Self {
            tools,
            prompt: prompt.into(),
            system: None,
            max_turns: None,
        }
    }

    /// Set the system prompt.
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Set the maximum number of LLM calls.
    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = Some(max_turns);
        self
    }
}

/// Result of running an agent.
#[derive(Debug, Clone)]
pub struct AgentResult {
    /// Final text response.
    pub output: String,
    /// Usage summary across the agent run.
    pub usage: AgentUsage,
    /// The final response from the model.
    pub response: Response,
}

/// Usage tracking across an agent run.
#[derive(Debug, Clone, Default)]
pub struct AgentUsage {
    /// Total input tokens consumed.
    pub input_tokens: u64,
    /// Total output tokens generated.
    pub output_tokens: u64,
    /// Total tokens (input + output).
    pub total_tokens: u64,
    /// Number of LLM API calls made.
    pub llm_calls: usize,
    /// Number of tool calls executed.
    pub tool_calls: usize,
}

impl Client {
    /// Performs a simple chat completion and returns the full Response.
    ///
    /// This is the most ergonomic way to get a response when you need access
    /// to metadata like usage, model, or stop reason.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use modelrelay::{Client, ChatOptions};
    ///
    /// let client = Client::from_secret_key("mr_sk_...")?.build()?;
    /// let response = client.chat("claude-sonnet-4-5", "What is 2 + 2?", None).await?;
    /// println!("{}", response.text());
    /// println!("{:?}", response.usage);
    /// ```
    pub async fn chat(
        &self,
        model: impl Into<Model>,
        prompt: impl Into<String>,
        options: Option<ChatOptions>,
    ) -> Result<Response> {
        let opts = options.unwrap_or_default();

        let mut builder = ResponseBuilder::new().model(model);

        if let Some(customer_id) = opts.customer_id {
            builder = builder.customer_id(customer_id);
        }

        if let Some(system) = opts.system {
            builder = builder.system(system);
        }

        builder = builder.user(prompt);

        builder.send(&self.responses()).await
    }

    /// Performs a simple prompt and returns just the text response.
    ///
    /// This is the most ergonomic way to get a quick answer.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use modelrelay::Client;
    ///
    /// let client = Client::from_secret_key("mr_sk_...")?.build()?;
    /// let answer = client.ask("claude-sonnet-4-5", "What is 2 + 2?", None).await?;
    /// println!("{}", answer); // "4"
    /// ```
    pub async fn ask(
        &self,
        model: impl Into<Model>,
        prompt: impl Into<String>,
        options: Option<ChatOptions>,
    ) -> Result<String> {
        let response = self.chat(model, prompt, options).await?;

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

    /// Runs an agentic tool loop to completion.
    ///
    /// Creates requests with the provided tools and runs until the model
    /// stops calling tools or max_turns is reached.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use modelrelay::{Client, AgentOptions, ToolBuilder};
    /// use serde::Deserialize;
    /// use schemars::JsonSchema;
    ///
    /// #[derive(JsonSchema, Deserialize)]
    /// struct ReadFileArgs {
    ///     /// Path to the file to read
    ///     path: String,
    /// }
    ///
    /// let tools = ToolBuilder::new()
    ///     .add_sync::<ReadFileArgs, _>("read_file", "Read a file", |args, _call| {
    ///         Ok(serde_json::json!({ "content": "file contents" }))
    ///     });
    ///
    /// let client = Client::from_secret_key("mr_sk_...")?.build()?;
    /// let result = client.agent(
    ///     "claude-sonnet-4-5",
    ///     AgentOptions::new(tools, "Read config.json and summarize it"),
    /// ).await?;
    ///
    /// println!("{}", result.output);
    /// ```
    pub async fn agent(
        &self,
        model: impl Into<Model>,
        options: AgentOptions,
    ) -> Result<AgentResult> {
        let model: Model = model.into();
        let max_turns = options.max_turns.unwrap_or(DEFAULT_MAX_TURNS);

        // Extract definitions and registry from ToolBuilder
        let (tool_definitions, tool_registry) = options.tools.build();

        let mut usage = AgentUsage::default();
        let mut input = Vec::new();

        // Build initial input
        if let Some(system) = options.system {
            input.push(crate::types::InputItem::system(system));
        }
        input.push(crate::types::InputItem::user(options.prompt));

        for _turn in 0..max_turns {
            // Build request
            let mut builder = ResponseBuilder::new()
                .model(model.clone())
                .input(input.clone());

            if !tool_definitions.is_empty() {
                builder = builder.tools(tool_definitions.clone());
            }

            // Make request
            let response = builder.send(&self.responses()).await?;

            usage.llm_calls += 1;
            usage.input_tokens += response.usage.input_tokens;
            usage.output_tokens += response.usage.output_tokens;
            usage.total_tokens += response.usage.total_tokens;

            // Get tool calls from response
            let tool_calls = get_all_tool_calls_from_response(&response);

            if tool_calls.is_empty() {
                // No tool calls, we're done
                return Ok(AgentResult {
                    output: response.text(),
                    usage,
                    response,
                });
            }

            // Execute tool calls
            usage.tool_calls += tool_calls.len();

            // Add assistant message with tool calls to history
            let assistant_text = response.text();
            input.push(assistant_message_with_tool_calls(
                assistant_text,
                tool_calls.clone(),
            ));

            // Execute tools and add results
            let results = tool_registry.execute_all(&tool_calls).await;
            for result in results {
                let result_content = format_tool_result(&result);
                input.push(tool_result_message(&result.tool_call_id, result_content));
            }
        }

        // Hit max turns without completion - this is an error
        Err(Error::AgentMaxTurns { max_turns })
    }
}

/// Formats a tool execution result as a string for sending back to the model.
fn format_tool_result(result: &ToolExecutionResult) -> String {
    if let Some(ref error) = result.error {
        format!("Error: {}", error)
    } else if let Some(ref value) = result.result {
        match value {
            serde_json::Value::String(s) => s.clone(),
            _ => serde_json::to_string(value).unwrap_or_default(),
        }
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_options_builder() {
        let opts = ChatOptions::new()
            .with_system("You are helpful")
            .with_customer_id("cust_123");

        assert_eq!(opts.system, Some("You are helpful".to_string()));
        assert_eq!(opts.customer_id, Some("cust_123".to_string()));
    }

    #[test]
    fn test_agent_options_builder() {
        let tools = ToolBuilder::new();
        let opts = AgentOptions::new(tools, "Hello")
            .with_system("Be helpful")
            .with_max_turns(50);

        assert_eq!(opts.prompt, "Hello");
        assert_eq!(opts.system, Some("Be helpful".to_string()));
        assert_eq!(opts.max_turns, Some(50));
    }

    #[test]
    fn test_get_all_tool_calls_empty() {
        let response = Response {
            id: "test".to_string(),
            model: "test".into(),
            output: vec![],
            usage: crate::types::Usage {
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
            },
            stop_reason: None,
            request_id: None,
            provider: None,
            citations: None,
        };

        let calls = get_all_tool_calls_from_response(&response);
        assert!(calls.is_empty());
    }
}
