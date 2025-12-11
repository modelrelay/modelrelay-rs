# ModelRelay Rust SDK

```toml
[dependencies]
modelrelay = "0.39.1"
```

## Top 3 Features

### 1. Structured Outputs with Auto-Retry

Define a Rust struct, and the SDK generates the JSON schema, validates responses, and automatically retries on malformed output:

```rust
use modelrelay::{Client, ChatRequestBuilder};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct Person {
    name: String,
    age: u32,
    email: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::with_key(std::env::var("MODELRELAY_API_KEY")?).build()?;

    let result = ChatRequestBuilder::new("claude-sonnet-4-20250514")
        .user("Extract: John Doe is 30 years old, john@example.com")
        .structured::<Person>()
        .max_retries(2)  // Auto-retry on validation errors
        .send(&client.llm())
        .await?;

    println!("{}: {} years old", result.value.name, result.value.age);
    Ok(())
}
```

### 2. Streaming Structured JSON with Field Completion

Stream typed JSON and know exactly when each field is complete for progressive UI rendering:

```rust
use futures_util::StreamExt;

#[derive(Debug, Deserialize, JsonSchema)]
struct Article {
    title: String,
    summary: String,
    body: String,
}

let mut stream = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write an article about Rust's ownership model")
    .structured::<Article>()
    .stream(&client.llm())
    .await?;

while let Some(event) = stream.next().await {
    let event = event?;

    // Render fields the moment they're complete
    if event.complete_fields.contains("title") {
        println!("Title: {}", event.payload.title);
    }
    if event.complete_fields.contains("summary") {
        println!("Summary: {}", event.payload.summary);
    }

    // Show typing indicator for incomplete body
    if !event.complete_fields.contains("body") {
        print!("\rBody: {}...", &event.payload.body[..50.min(event.payload.body.len())]);
    }
}
```

### 3. Tool Use with Type-Safe Argument Parsing

Register tools with typed argument validation and automatic retry on malformed calls:

```rust
use modelrelay::{
    Tool, ToolRegistry, ToolChoice, ChatRequestBuilder,
    parse_and_validate_tool_args, ValidateArgs, execute_with_retry, RetryOptions,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct WeatherArgs {
    location: String,
    unit: Option<String>,
}

impl ValidateArgs for WeatherArgs {
    fn validate(&self) -> Result<(), String> {
        if self.location.is_empty() {
            return Err("location cannot be empty".into());
        }
        Ok(())
    }
}

// Type-safe argument parsing with validation
let registry = ToolRegistry::new()
    .register("get_weather", modelrelay::sync_handler(|args, _call| {
        let weather: WeatherArgs = parse_and_validate_tool_args(&args)?;
        Ok(serde_json::json!({
            "temp": 72,
            "unit": weather.unit.unwrap_or("fahrenheit".into()),
            "location": weather.location
        }))
    }));

// Define the tool schema from the type
let weather_tool = Tool::function(
    "get_weather",
    "Get current weather for a location",
    modelrelay::function_tool_from_type::<WeatherArgs>()?.parameters,
);

// Send request with tools
let response = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("What's the weather in San Francisco?")
    .tools(vec![weather_tool])
    .tool_choice(ToolChoice::auto())
    .send(&client.llm())
    .await?;

// Execute with auto-retry on malformed arguments
if let Some(tool_calls) = response.tool_calls {
    let results = execute_with_retry(
        &registry,
        tool_calls,
        RetryOptions {
            max_retries: 2,
            on_retry: |error_msgs, _attempt| Box::pin(async move {
                // Send errors back to model to get corrected calls
                // (simplified - real impl would call the LLM)
                Ok(vec![])
            }),
        },
    ).await;

    for result in results {
        println!("Tool result: {:?}", result.result);
    }
}
```

## Quick Start

### Basic Chat

```rust
use modelrelay::{Client, ChatRequestBuilder};

let client = Client::with_key(std::env::var("MODELRELAY_API_KEY")?).build()?;

let response = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .system("You are a helpful assistant.")
    .user("Hello!")
    .send(&client.llm())
    .await?;

println!("{}", response.content.join(""));
```

### Streaming

```rust
use futures_util::StreamExt;

let mut stream = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write a haiku about Rust")
    .stream(&client.llm())
    .await?;

while let Some(chunk) = stream.next().await {
    if let Some(text) = chunk?.text_delta {
        print!("{}", text);
    }
}
```

### Customer-Attributed Requests

For metered billing, use `CustomerChatRequestBuilder` - the customer's tier determines the model:

```rust
use modelrelay::CustomerChatRequestBuilder;

// Basic chat - model determined by customer's tier
let response = CustomerChatRequestBuilder::new("customer-123")
    .user("Hello!")
    .send(&client.llm())
    .await?;

// Structured output (new in 0.39)
let result = CustomerChatRequestBuilder::new("customer-123")
    .user("Extract: John Doe is 30, john@example.com")
    .structured::<Person>()
    .max_retries(2)
    .send(&client.llm())
    .await?;

// Streaming
let mut stream = CustomerChatRequestBuilder::new("customer-123")
    .user("Write a haiku about Rust")
    .stream(&client.llm())
    .await?;

while let Some(chunk) = stream.next().await {
    if let Some(text) = chunk?.text_delta {
        print!("{}", text);
    }
}
```

### Blocking API (No Tokio)

```rust
use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder};

let client = BlockingClient::new(BlockingConfig {
    api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
    ..Default::default()
})?;

let response = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Hello!")
    .send_blocking(&client.llm())?;
```

## API Matrix

Both `ChatRequestBuilder` and `CustomerChatRequestBuilder` support all modes:

| Mode | Non-Streaming | Streaming | Structured | Structured Streaming |
|------|---------------|-----------|------------|---------------------|
| **Async** | `.send()` | `.stream()` | `.structured::<T>().send()` | `.structured::<T>().stream()` |
| **Blocking** | `.send_blocking()` | `.stream_blocking()` | `.structured::<T>().send_blocking()` | `.structured::<T>().stream_blocking()` |

Customer structured output support added in 0.39.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `client` | Yes | Async client with Tokio |
| `streaming` | Yes | NDJSON streaming support |
| `blocking` | No | Sync client without Tokio |
| `tracing` | No | OpenTelemetry spans/events |
| `mock` | No | In-memory client for tests |

## Error Handling

API errors include typed helpers for common cases:

```rust
use modelrelay::Error;

match client.llm().chat(request).await {
    Ok(response) => println!("{}", response.content.join("")),
    Err(Error::Api(e)) if e.is_rate_limit() => {
        // Back off and retry
    }
    Err(Error::Api(e)) if e.is_unauthorized() => {
        // Invalid or expired API key
    }
    Err(Error::Transport(e)) => {
        // Network/connectivity issues
    }
    Err(e) => return Err(e.into()),
}
```
