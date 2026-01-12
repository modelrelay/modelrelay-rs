//! Example demonstrating the convenience API for quick prototyping
//! and simple use cases.
//!
//! Run with:
//! ```bash
//! MODELRELAY_API_KEY=mr_sk_... cargo run --example convenience
//! ```

use modelrelay::{AgentOptions, ChatOptions, Client, ToolBuilder};
use schemars::JsonSchema;
use serde::Deserialize;
use std::error::Error;

#[derive(Debug, Deserialize, JsonSchema)]
struct CalculateArgs {
    /// Math expression to evaluate (e.g., "2 + 2")
    expression: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let api_key = std::env::var("MODELRELAY_API_KEY")
        .expect("MODELRELAY_API_KEY environment variable must be set");

    let client = Client::from_api_key(api_key)?.build()?;

    // Example 1: Ask — Get a quick answer
    println!("=== Ask Example ===");
    let answer = client
        .ask("claude-sonnet-4-5", "What is 2 + 2?", None)
        .await?;
    println!("Answer: {}", answer);
    println!();

    // Example 2: Chat — Full response with metadata
    println!("=== Chat Example ===");
    let response = client
        .chat(
            "claude-sonnet-4-5",
            "Explain the concept of recursion in one sentence.",
            Some(
                ChatOptions::new()
                    .with_system("You are a computer science teacher. Be concise."),
            ),
        )
        .await?;
    println!("Response: {}", response.text());
    println!(
        "Tokens: {} input, {} output, {} total",
        response.usage.input_tokens, response.usage.output_tokens, response.usage.total_tokens
    );
    println!();

    // Example 3: Agent — Agentic tool loop
    println!("=== Agent Example ===");

    let tools = ToolBuilder::new().add_sync::<CalculateArgs, _>(
        "calculate",
        "Evaluate a simple math expression",
        |args, _call| {
            // In a real app, you'd use a proper expression parser
            // This is just a demo returning a mock result
            Ok(serde_json::json!({
                "expression": args.expression,
                "result": "42",
                "note": "Demo calculator - always returns 42"
            }))
        },
    );

    let result = client
        .agent(
            "claude-sonnet-4-5",
            AgentOptions::new(tools, "Calculate 6 * 7 using the calculate tool.")
                .with_system("You are a helpful math assistant. Use the calculate tool when asked to compute expressions."),
        )
        .await?;

    println!("Agent output: {}", result.output);
    println!(
        "Agent usage: {} LLM calls, {} tool calls",
        result.usage.llm_calls, result.usage.tool_calls
    );

    Ok(())
}
