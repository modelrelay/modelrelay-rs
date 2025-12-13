# ModelRelay Rust SDK

```toml
[dependencies]
modelrelay = "0.44.0"
```

## Top 3 Features

### 1. Structured Outputs with Auto-Retry

Define a Rust struct, and the SDK generates the JSON schema, validates responses, and automatically retries on malformed output:

```rust
use modelrelay::{Client, ResponseBuilder};
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

    let result = ResponseBuilder::new()
        .model("claude-sonnet-4-20250514")
        .user("Extract: John Doe is 30 years old, john@example.com")
        .structured::<Person>()
        .max_retries(2)
        .send(&client.responses())
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
let mut stream = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Write an article about Rust's ownership model")
    .structured::<Article>()
    .stream(&client.responses())
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
let article: Article = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Write an article about Rust")
    .structured::<Article>()
    .stream(&client.responses())
    .await?
    .collect()
    .await?;
```

### 3. Tool Use with Type-Safe Argument Parsing

Register tools with typed argument validation:

```rust
use modelrelay::{Tool, ToolRegistry, ToolChoice, ResponseBuilder, function_tool_from_type};
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
let response = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("What's the weather in San Francisco?")
    .tools(vec![weather_tool])
    .tool_choice(ToolChoice::auto())
    .send(&client.responses())
    .await?;

// Execute tool calls
let tool_calls: Vec<modelrelay::ToolCall> = response
    .output
    .iter()
    .flat_map(|item| match item {
        modelrelay::OutputItem::Message { tool_calls, .. } => tool_calls.clone().unwrap_or_default(),
    })
    .collect();

if !tool_calls.is_empty() {
    let results = registry.execute_all(&tool_calls).await;
    for result in results {
        println!("Tool result: {:?}", result.result);
    }
}
```

## Quick Start

### Basic Response

```rust
use modelrelay::{Client, ResponseBuilder};

let client = Client::with_key(std::env::var("MODELRELAY_API_KEY")?).build()?;

let response = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .system("You are a helpful assistant.")
    .user("Hello!")
    .send(&client.responses())
    .await?;

let mut text = String::new();
for item in &response.output {
    let modelrelay::OutputItem::Message { role, content, .. } = item;
    if *role != modelrelay::MessageRole::Assistant {
        continue;
    }
    for part in content {
        let modelrelay::ContentPart::Text { text: t } = part;
        text.push_str(t);
    }
}
println!("{}", text);
```

### Streaming

```rust
use futures_util::StreamExt;

// Simple: iterate text deltas directly
let mut deltas = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Write a haiku about Rust")
    .stream_deltas(&client.responses())
    .await?;

while let Some(text) = deltas.next().await {
    print!("{}", text?);
}

// Or use the raw stream for more control
let mut stream = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Write a haiku about Rust")
    .stream(&client.responses())
    .await?;

while let Some(event) = stream.next().await {
    if let Some(text) = event?.text_delta {
        print!("{}", text);
    }
}
```

### Customer-Attributed Requests

For metered billing, set `customer_id(...)` - the customer's tier determines the model and `model(...)` can be omitted:

```rust
use modelrelay::ResponseBuilder;

// Basic response - model determined by customer's tier
let response = ResponseBuilder::new()
    .customer_id("customer-123")
    .user("Hello!")
    .send(&client.responses())
    .await?;

// Structured output
let result = ResponseBuilder::new()
    .customer_id("customer-123")
    .user("Extract: John Doe is 30, john@example.com")
    .structured::<Person>()
    .max_retries(2)
    .send(&client.responses())
    .await?;

// Streaming text deltas
let mut deltas = ResponseBuilder::new()
    .customer_id("customer-123")
    .user("Write a haiku")
    .stream_deltas(&client.responses())
    .await?;

while let Some(text) = deltas.next().await {
    print!("{}", text?);
}
```

### Blocking API (No Tokio)

```rust
use modelrelay::{BlockingClient, BlockingConfig, ResponseBuilder};

let client = BlockingClient::new(BlockingConfig {
    api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
    ..Default::default()
})?;

// Non-streaming
let response = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Hello!")
    .send_blocking(&client.responses())?;

// Streaming text deltas
let deltas = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Write a haiku")
    .stream_deltas_blocking(&client.responses())?;

for text in deltas {
    print!("{}", text?);
}
```

## API Matrix

`ResponseBuilder` supports all modes (use `customer_id(...)` for customer-attributed calls):

| Mode | Non-Streaming | Streaming | Text Deltas | Structured | Structured Streaming |
|------|---------------|-----------|-------------|------------|---------------------|
| **Async** | `.send()` | `.stream()` | `.stream_deltas()` | `.structured::<T>().send()` | `.structured::<T>().stream()` |
| **Blocking** | `.send_blocking()` | `.stream_blocking()` | `.stream_deltas_blocking()` | `.structured::<T>().send_blocking()` | `.structured::<T>().stream_blocking()` |

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
use modelrelay::{Error, ResponseBuilder};

let result = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Hello!")
    .send(&client.responses())
    .await;

match result {
    Ok(_response) => println!("ok"),
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
