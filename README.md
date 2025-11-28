# ModelRelay Rust SDK

Async and blocking clients for the ModelRelay API with optional SSE streaming, tracing, and offline mocks.

## Install

```toml
[dependencies]
modelrelay = "0.3.5"
# blocking-only:
# modelrelay = { version = "0.3.5", default-features = false, features = ["blocking"] }
# blocking with streaming (no Tokio runtime):
# modelrelay = { version = "0.3.5", default-features = false, features = ["blocking", "streaming"] }
# async without streaming:
# modelrelay = { version = "0.3.5", default-features = false, features = ["client"] }
```

### Features
- `client` (default): async reqwest client + Tokio.
- `blocking`: blocking reqwest client (no Tokio).
- `streaming` (default): SSE streaming for `/llm/proxy`.
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

    let request = ProxyRequest::builder(Model::OpenAIGpt4o)
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

Request JSON or JSON Schema-constrained responses when the provider supports it:

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

let request = ProxyRequest::builder("openai/gpt-4o-mini")
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
        ProxyRequest::builder("openai/gpt-4o-mini").user("hi").build()?,
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
- Auth/frontend tokens and API keys: [`docs/auth.md`](docs/auth.md)

## Quickstart (blocking/CLI)

```rust
use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder, ChatStreamAdapter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let mut adapter = ChatStreamAdapter::new(
        ChatRequestBuilder::new("openai/gpt-4o-mini")
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
- Predefined environments: production/staging/sandbox or custom base URL. Staging base: `https://api-stg.modelrelay.ai/api/v1`.

## Environment variables
- `MODELRELAY_API_KEY` — secret key for server-to-server calls.
- `MODELRELAY_PUBLISHABLE_KEY` — publishable key for frontend token exchange.
- `MODELRELAY_BASE_URL` — override API base URL.
