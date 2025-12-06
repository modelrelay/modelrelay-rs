# ModelRelay Rust SDK

Async and blocking clients for the ModelRelay API with optional SSE streaming, tracing, and offline mocks.

## Install

```toml
[dependencies]
modelrelay = "0.8.0"
# blocking-only:
# modelrelay = { version = "0.8.0", default-features = false, features = ["blocking"] }
# blocking with streaming (no Tokio runtime):
# modelrelay = { version = "0.8.0", default-features = false, features = ["blocking", "streaming"] }
# async without streaming:
# modelrelay = { version = "0.8.0", default-features = false, features = ["client"] }
```

### Features
- `client` (default): async reqwest client + Tokio.
- `blocking`: blocking reqwest client (no Tokio).
- `streaming` (default): SSE streaming for `/llm/proxy`; opt into NDJSON framing with `ProxyOptions::with_ndjson_stream()` or `ChatRequestBuilder::ndjson_stream()`.
- `tracing`: optional spans/events around HTTP + streaming.
- `mock`: in-memory mock client + fixtures for offline tests.

## Quickstart (async)

```rust
use modelrelay::{Client, Config, Model, ProxyOptions, ProxyRequest};

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
use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder, ChatStreamAdapter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let mut adapter = ChatStreamAdapter::new(
        ChatRequestBuilder::new("gpt-4o-mini")
            .message("user", "Tell me a short fact about Rust.")
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
