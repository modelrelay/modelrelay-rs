# ModelRelay Rust SDK

```toml
[dependencies]
modelrelay = "0.41.0"
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
        .max_retries(2)
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
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct Article {
    title: String,
    summary: String,
    body: String,
}

// Stream with field completion tracking
let mut stream = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write an article about Rust's ownership model")
    .structured::<Article>()
    .stream(&client.llm())
    .await?;

while let Some(event) = stream.next().await {
    let event = event?;

    // Render fields the moment they're complete
    for field in &event.complete_fields {
        match field.as_str() {
            "title" => println!("Title: {}", event.payload.title),
            "summary" => println!("Summary: {}", event.payload.summary),
            _ => {}
        }
    }
}

// Or just collect the final result
let article: Article = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write an article about Rust")
    .structured::<Article>()
    .stream(&client.llm())
    .await?
    .collect()
    .await?;
```

### 3. Tool Use with Type-Safe Argument Parsing

Register tools with typed argument validation:

```rust
use modelrelay::{Tool, ToolRegistry, ToolChoice, ChatRequestBuilder, function_tool_from_type};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct WeatherArgs {
    location: String,
    #[serde(default)]
    unit: Option<String>,
}

// Register handler with automatic argument parsing
let registry = ToolRegistry::new()
    .register("get_weather", modelrelay::sync_handler(|args, _call| {
        let args: WeatherArgs = modelrelay::parse_tool_args(&args)?;
        Ok(serde_json::json!({
            "temp": 72,
            "location": args.location,
            "unit": args.unit.unwrap_or_else(|| "fahrenheit".into())
        }))
    }));

// Generate tool schema from the type
let weather_tool = Tool::function(
    "get_weather",
    "Get current weather for a location",
    function_tool_from_type::<WeatherArgs>()?.parameters,
);

// Send request with tools
let response = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("What's the weather in San Francisco?")
    .tools(vec![weather_tool])
    .tool_choice(ToolChoice::auto())
    .send(&client.llm())
    .await?;

// Execute tool calls
if let Some(tool_calls) = response.tool_calls {
    let results = registry.execute_all(tool_calls).await;
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

// Simple: iterate text deltas directly
let mut deltas = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write a haiku about Rust")
    .stream_deltas(&client.llm())
    .await?;

while let Some(text) = deltas.next().await {
    print!("{}", text?);
}

// Or use the raw stream for more control
let mut stream = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write a haiku about Rust")
    .stream(&client.llm())
    .await?;

while let Some(event) = stream.next().await {
    if let Some(text) = event?.text_delta {
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

// Structured output
let result = CustomerChatRequestBuilder::new("customer-123")
    .user("Extract: John Doe is 30, john@example.com")
    .structured::<Person>()
    .max_retries(2)
    .send(&client.llm())
    .await?;

// Streaming
let mut stream = CustomerChatRequestBuilder::new("customer-123")
    .user("Write a haiku")
    .stream(&client.llm())
    .await?;

while let Some(event) = stream.next().await {
    if let Some(text) = event?.text_delta {
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

// Non-streaming
let response = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Hello!")
    .send_blocking(&client.llm())?;

// Streaming text deltas
let deltas = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write a haiku")
    .stream_deltas_blocking(&client.llm())?;

for text in deltas {
    print!("{}", text?);
}
```

## API Matrix

**ChatRequestBuilder** (model-specified requests):

| Mode | Non-Streaming | Streaming | Text Deltas | Structured | Structured Streaming |
|------|---------------|-----------|-------------|------------|---------------------|
| **Async** | `.send()` | `.stream()` | `.stream_deltas()` | `.structured::<T>().send()` | `.structured::<T>().stream()` |
| **Blocking** | `.send_blocking()` | `.stream_blocking()` | `.stream_deltas_blocking()` | `.structured::<T>().send_blocking()` | `.structured::<T>().stream_blocking()` |

**CustomerChatRequestBuilder** (tier-determined model):

| Mode | Non-Streaming | Streaming | Structured | Structured Streaming |
|------|---------------|-----------|------------|---------------------|
| **Async** | `.send()` | `.stream()` | `.structured::<T>().send()` | `.structured::<T>().stream()` |
| **Blocking** | `.send_blocking()` | `.stream_blocking()` | `.structured::<T>().send_blocking()` | `.structured::<T>().stream_blocking()` |

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
use modelrelay::{Error, ChatRequestBuilder};

let result = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Hello!")
    .send(&client.llm())
    .await;

match result {
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
