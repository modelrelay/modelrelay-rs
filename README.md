# ModelRelay Rust SDK

Async and blocking clients for the ModelRelay API with optional SSE streaming, tracing, and offline mocks.

## Install

```toml
[dependencies]
modelrelay = "0.23.0"
# blocking-only:
# modelrelay = { version = "0.23.0", default-features = false, features = ["blocking"] }
# blocking with streaming (no Tokio runtime):
# modelrelay = { version = "0.23.0", default-features = false, features = ["blocking", "streaming"] }
# async without streaming:
# modelrelay = { version = "0.23.0", default-features = false, features = ["client"] }
```

### Features
- `client` (default): async reqwest client + Tokio.
- `blocking`: blocking reqwest client (no Tokio).
- `streaming` (default): SSE streaming for `/llm/proxy`; opt into NDJSON framing with `ProxyOptions::with_ndjson_stream()` or `ChatRequestBuilder::ndjson_stream()`.
- `tracing`: optional spans/events around HTTP + streaming.
- `mock`: in-memory mock client + fixtures for offline tests.

## Quickstart (async)

```rust
use modelrelay::{Client, Config, MessageRole, Model, ProxyOptions, ProxyRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest::builder(Model::Gpt4o)
        .user("Write a short greeting.")
        .max_tokens(64)
        .build()?;

    let completion = client
        .llm()
        .proxy(request, ProxyOptions::default().with_request_id("chat-1"))
        .await?;

    println!("response {}: {}", completion.id, completion.content.join(""));
    Ok(())
}
```

### Message roles

Use the typed `MessageRole` enum instead of strings:

```rust
use modelrelay::{ChatRequestBuilder, MessageRole};

// Using typed role constants
let request = ChatRequestBuilder::new("gpt-4o")
    .message(MessageRole::System, "You are helpful.")
    .message(MessageRole::User, "Hello!")
    .build_request()?;

// Or use convenience methods (recommended)
let request = ChatRequestBuilder::new("gpt-4o")
    .system("You are helpful.")
    .user("Hello!")
    .build_request()?;

// Available roles: MessageRole::User, Assistant, System, Tool
```

### Customer-attributed requests

For customer-attributed requests, the customer's tier determines which model to use.
Use `for_customer()` instead of providing a model:

```rust
use modelrelay::{Client, Config, CustomerChatRequestBuilder};

async fn customer_chat() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    // Customer-attributed: tier determines model, no model parameter needed
    let response = client.llm()
        .for_customer("customer-123")
        .system("You are a helpful assistant.")
        .user("Hello!")
        .send(&client.llm())
        .await?;

    println!("response: {}", response.content.join(""));

    // Streaming works the same way
    let stream = client.llm()
        .for_customer("customer-123")
        .user("Tell me a joke")
        .stream(&client.llm())
        .await?;

    Ok(())
}
```

This provides compile-time separation between:
- **Direct/PAYGO requests** (`ChatRequestBuilder::new(model)`) — model is required
- **Customer-attributed requests** (`for_customer(id)`) — tier determines model

### Structured outputs (response_format)

Request JSON or JSON Schema-constrained responses when the backend supports it:

```rust
use modelrelay::{Client, Config, ProxyOptions, ProxyRequest, ResponseFormat, ResponseFormatKind, ResponseJSONSchema};

# async fn demo() -> anyhow::Result<()> {
let client = Client::new(Config { api_key: Some("sk_...".into()), ..Default::default() })?;

let format = ResponseFormat {
    kind: ResponseFormatKind::JsonSchema,
    json_schema: Some(ResponseJSONSchema {
        name: "summary".into(),
        description: Some("short JSON summary".into()),
        schema: serde_json::json!({"type": "object", "properties": {"headline": {"type": "string"}}}),
        strict: Some(true),
    }),
};

let request = ProxyRequest::builder("gpt-4o-mini")
    .user("Summarize ModelRelay")
    .response_format(format)
    .build()?;

let resp = client.llm().proxy(request, ProxyOptions::default()).await?;
println!("json: {}", resp.content.join(""));
# Ok(())
# }
```

### Structured streaming (NDJSON + response_format)

Use the structured streaming contract for `/llm/proxy` to stream schema-valid
JSON payloads over NDJSON:

```rust
use modelrelay::{
    Client, Config, ChatRequestBuilder, ProxyOptions, ResponseFormat, ResponseFormatKind, ResponseJSONSchema,
};

#[derive(Debug, serde::Deserialize)]
struct Item {
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct RecommendationPayload {
    items: Vec<Item>,
}

# async fn demo() -> anyhow::Result<()> {
let client = Client::new(Config { api_key: Some("sk_...".into()), ..Default::default() })?;

let format = ResponseFormat {
    kind: ResponseFormatKind::JsonSchema,
    json_schema: Some(ResponseJSONSchema {
        name: "recommendations".into(),
        description: Some("AI-generated item recommendations".into()),
        schema: serde_json::json!({"type":"object","properties":{"items":{"type":"array"}}}),
        strict: Some(true),
    }),
};

let stream = ChatRequestBuilder::new("grok-4-1-fast")
    .user("Recommend items for my user")
    .response_format(format)
    .stream_json::<RecommendationPayload>(&client.llm())
    .await?;

// Progressive UI: handle updates and final completion.
let mut stream = stream;
while let Some(evt) = stream.next().await? {
    match evt.kind {
        modelrelay::StructuredRecordKind::Update => {
            println!("partial items: {}", evt.payload.items.len());
        }
        modelrelay::StructuredRecordKind::Completion => {
            println!("final items: {}", evt.payload.items.len());
        }
    }
}
# Ok(())
# }
```

### Type-safe structured outputs with automatic schema inference

For automatic schema generation and validation, use the `structured()` builder
with types that derive `JsonSchema`:

```rust
use modelrelay::{Client, Config, ChatRequestBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Derive JsonSchema for automatic schema generation
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Person {
    name: String,
    age: u32,
}

# async fn demo() -> anyhow::Result<()> {
let client = Client::new(Config {
    api_key: Some("sk_...".into()),
    ..Default::default()
})?;

// structured::<T>() auto-generates the schema and validates responses
let result = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Extract: John Doe is 30 years old")
    .structured::<Person>()
    .max_retries(2) // Retry on validation failures
    .send(&client.llm())
    .await?;

println!("Name: {}, Age: {}", result.value.name, result.value.age);
println!("Succeeded on attempt {}", result.attempts);
# Ok(())
# }
```

#### Schema features with schemars

The `schemars` crate maps Rust types to JSON Schema:

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Status {
    // Required field
    code: String,

    // Optional field (not in "required" array)
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,

    // Description for documentation
    #[schemars(description = "User's email address")]
    email: String,

    // Enum constraint
    priority: Priority,

    // Nested objects are fully supported
    address: Address,

    // Arrays
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Priority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Address {
    city: String,
    country: String,
}
```

#### Handling validation errors

When validation fails after all retries:

```rust
use modelrelay::{ChatRequestBuilder, StructuredError};

# async fn demo(client: &modelrelay::LLMClient) -> anyhow::Result<()> {
let result = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Extract person info")
    .structured::<Person>()
    .max_retries(2)
    .send(client)
    .await;

match result {
    Ok(r) => println!("Success: {:?}", r.value),
    Err(StructuredError::Exhausted(e)) => {
        println!("Failed after {} attempts", e.all_attempts.len());
        for attempt in &e.all_attempts {
            println!("Attempt {}: {}", attempt.attempt, attempt.raw_json);
            match &attempt.error {
                modelrelay::StructuredErrorKind::Validation { issues } => {
                    for issue in issues {
                        println!("  - {}: {}",
                            issue.path.as_deref().unwrap_or("root"),
                            issue.message
                        );
                    }
                }
                modelrelay::StructuredErrorKind::Decode { message } => {
                    println!("  Decode error: {}", message);
                }
            }
        }
    }
    Err(e) => println!("Other error: {}", e),
}
# Ok(())
# }
```

#### Custom retry handlers

Customize retry behavior with your own handler:

```rust
use modelrelay::{ChatRequestBuilder, RetryHandler, StructuredErrorKind, ProxyMessage, MessageRole};

struct MyRetryHandler;

impl RetryHandler for MyRetryHandler {
    fn on_validation_error(
        &self,
        attempt: u32,
        _raw_json: &str,
        error: &StructuredErrorKind,
        _messages: &[ProxyMessage],
    ) -> Option<Vec<ProxyMessage>> {
        if attempt >= 3 {
            return None; // Stop retrying
        }
        Some(vec![ProxyMessage {
            role: MessageRole::User,
            content: format!("Invalid response: {:?}. Try again.", error),
            tool_calls: None,
            tool_call_id: None,
        }])
    }
}

# async fn demo(client: &modelrelay::LLMClient) -> anyhow::Result<()> {
let result = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Extract person info")
    .structured::<Person>()
    .max_retries(3)
    .retry_handler(MyRetryHandler)
    .send(client)
    .await?;
# Ok(())
# }
```

#### Streaming structured outputs

For streaming with schema inference (no retries):

```rust
# async fn demo(client: &modelrelay::LLMClient) -> anyhow::Result<()> {
let mut stream = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Extract: Jane, 25")
    .structured::<Person>()
    .stream(client)
    .await?;

while let Some(evt) = stream.next().await? {
    if evt.kind == modelrelay::StructuredRecordKind::Completion {
        println!("Final: {:?}", evt.payload);
    }
}
# Ok(())
# }
```

### Error handling

```rust
use modelrelay::{Client, Config, Error, ProxyOptions, ProxyRequest};

async fn call() -> Result<(), Error> {
    let client = Client::new(Config {
        api_key: Some("sk_...".into()),
        ..Default::default()
    })?;

    match client.llm().proxy(
        ProxyRequest::builder("gpt-4o-mini").user("hi").build()?,
        ProxyOptions::default(),
    ).await {
        Ok(resp) => println!("reply: {}", resp.content.join("")),
        Err(Error::Validation(err)) => eprintln!("bad request: {}", err),
        Err(Error::Api(err)) => eprintln!("status {} body {:?}", err.status, err.raw_body),
        Err(Error::Transport(err)) => eprintln!("network: {}", err),
        Err(other) => eprintln!("unexpected: {}", other),
    }
    Ok(())
}
```

## More examples
- Async streaming + chat builder: [`docs/async.md`](docs/async.md)
- Blocking usage (streaming + non-streaming): [`docs/blocking.md`](docs/blocking.md)
- Tracing + metrics hooks: [`docs/telemetry.md`](docs/telemetry.md)
- Offline tests with mocks/fixtures: [`docs/mocks.md`](docs/mocks.md)
- Auth & API keys: [`docs/auth.md`](docs/auth.md)

## Quickstart (blocking/CLI)

```rust
use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder, ChatStreamAdapter, MessageRole};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let mut adapter = ChatStreamAdapter::new(
        ChatRequestBuilder::new("gpt-4o-mini")
            .user("Tell me a short fact about Rust.")  // convenience method
            .request_id("cli-chat-1")
            .stream_blocking(&client.llm())?,
    );

    while let Some(delta) = adapter.next_delta()? {
        print!("{delta}");
    }
    Ok(())
}
```

## Configuration highlights
- Defaults: 5s connect timeout, 60s request timeout (non-streaming), 3 attempts with jittered exponential backoff.
- Per-request overrides via `ProxyOptions` (`timeout`, `retry`, extra headers/metadata, request IDs).
- Custom base URL via `base_url` config option (defaults to production API).

## Environment variables
- `MODELRELAY_API_KEY` — secret key for server-to-server calls.
- `MODELRELAY_BASE_URL` — override API base URL.

## Backend Customer Management

Use a secret key (`mr_sk_*`) to manage customers from your backend:

```rust
use modelrelay::{Client, Config, CustomerCreateRequest, CustomerUpsertRequest, CheckoutSessionRequest};

async fn manage_customers() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?), // mr_sk_xxx
        ..Default::default()
    })?;

    // Create or update a customer (upsert by external_id)
    let customer = client.customers().upsert(CustomerUpsertRequest {
        tier_id: "your-tier-uuid".into(),
        external_id: "github-user-12345".into(),  // your app's user ID
        email: Some("user@example.com".into()),
        metadata: None,
    }).await?;

    // List all customers
    let customers = client.customers().list().await?;

    // Get a specific customer
    let customer = client.customers().get("customer-uuid").await?;

    // Create a checkout session for subscription billing
    let session = client.customers().create_checkout_session(
        "customer-uuid",
        CheckoutSessionRequest {
            success_url: "https://myapp.com/billing/success".into(),
            cancel_url: "https://myapp.com/billing/cancel".into(),
        },
    ).await?;
    // Redirect user to session.url to complete payment

    // Check subscription status
    let status = client.customers().get_subscription("customer-uuid").await?;
    if status.active {
        // Grant access
    }

    // Delete a customer
    client.customers().delete("customer-uuid").await?;

    Ok(())
}
```
