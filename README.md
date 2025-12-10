# ModelRelay Rust SDK

```toml
[dependencies]
modelrelay = "0.27.0"
```

## Streaming Chat

```rust
use modelrelay::{Client, Config, ChatRequestBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let mut stream = ChatRequestBuilder::new("claude-sonnet-4-20250514")
        .system("You are helpful.")
        .user("Hello!")
        .stream(&client.llm())
        .await?;

    while let Some(chunk) = stream.next().await? {
        if let Some(delta) = chunk.text_delta {
            print!("{}", delta);
        }
    }
    Ok(())
}
```

## Structured Outputs

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Person {
    name: String,
    age: u32,
}

let result = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Extract: John Doe is 30 years old")
    .structured::<Person>()
    .max_retries(2)
    .send(&client.llm())
    .await?;

println!("Name: {}, Age: {}", result.value.name, result.value.age);
```

## Streaming Structured Outputs

Build progressive UIs that render fields as they complete:

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Article {
    title: String,
    summary: String,
    body: String,
}

let mut stream = ChatRequestBuilder::new("claude-sonnet-4-20250514")
    .user("Write an article about Rust")
    .stream_structured::<Article>(&client.llm())
    .await?;

while let Some(event) = stream.next().await? {
    // Render fields as soon as they're complete
    if event.complete_fields.contains("title") {
        render_title(&event.payload.title);  // Safe to display
    }
    if event.complete_fields.contains("summary") {
        render_summary(&event.payload.summary);
    }

    // Show streaming preview of incomplete fields
    if !event.complete_fields.contains("body") {
        render_body_preview(&format!("{}▋", event.payload.body));
    }
}
```

## Customer-Attributed Requests

For metered billing, use `for_customer()` — the customer's tier determines the model:

```rust
let response = client.llm()
    .for_customer("customer-123")
    .user("Hello!")
    .send(&client.llm())
    .await?;
```

## Customer Management (Backend)

```rust
// Create/update customer
let customer = client.customers().upsert(CustomerUpsertRequest {
    tier_id: "tier-uuid".into(),
    external_id: "your-user-id".into(),
    email: Some("user@example.com".into()),
    metadata: None,
}).await?;

// Create checkout session for subscription billing
let session = client.customers().create_checkout_session(
    "customer-uuid",
    CheckoutSessionRequest {
        success_url: "https://myapp.com/success".into(),
        cancel_url: "https://myapp.com/cancel".into(),
    },
).await?;

// Check subscription status
let status = client.customers().get_subscription("customer-uuid").await?;
```

## Configuration

```rust
let client = Client::new(Config {
    api_key: Some("mr_sk_...".into()),
    connect_timeout: Some(Duration::from_secs(5)),
    retry: Some(RetryConfig { max_attempts: 3, ..Default::default() }),
    ..Default::default()
})?;
```

## Features

- `client` (default): async reqwest + Tokio
- `blocking`: blocking client (no Tokio)
- `streaming` (default): SSE streaming
- `tracing`: spans/events for observability
- `mock`: in-memory client for tests
