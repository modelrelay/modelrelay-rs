# ModelRelay Rust SDK

Async and blocking clients for the ModelRelay API with optional SSE streaming, tracing, and offline mocks.

## Install

```toml
[dependencies]
modelrelay = "0.2.1"
# blocking-only:
# modelrelay = { version = "0.2.1", default-features = false, features = ["blocking"] }
# async without streaming:
# modelrelay = { version = "0.2.1", default-features = false, features = ["client"] }
```

### Features
- `client` (default): async reqwest client + Tokio.
- `blocking`: blocking reqwest client (no Tokio).
- `streaming` (default): SSE streaming for `/llm/proxy`.
- `tracing`: optional spans/events around HTTP + streaming.
- `mock`: in-memory mock client + fixtures for offline tests.

## Quickstart (async)

```rust
use modelrelay::{Client, Config, Model, ProxyMessage, ProxyOptions, ProxyRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest::new(
        Model::OpenAIGpt4o,
        vec![ProxyMessage {
            role: "user".into(),
            content: "Write a short greeting.".into(),
        }],
    )?;

    let completion = client
        .llm()
        .proxy(request, ProxyOptions::default().with_request_id("chat-1"))
        .await?;

    println!("response {}: {}", completion.id, completion.content.join(""));
    Ok(())
}
```

## More examples
- Async streaming + chat builder: [`docs/async.md`](docs/async.md)
- Blocking usage (streaming + non-streaming): [`docs/blocking.md`](docs/blocking.md)
- Tracing + metrics hooks: [`docs/telemetry.md`](docs/telemetry.md)
- Offline tests with mocks/fixtures: [`docs/mocks.md`](docs/mocks.md)
- Auth/frontend tokens and API keys: [`docs/auth.md`](docs/auth.md)

## Configuration highlights
- Defaults: 5s connect timeout, 60s request timeout (non-streaming), 3 attempts with jittered exponential backoff.
- Per-request overrides via `ProxyOptions` (`timeout`, `retry`, extra headers/metadata, request IDs).
- Predefined environments: production/staging/sandbox or custom base URL.

## Environment variables
- `MODELRELAY_API_KEY` — secret key for server-to-server calls.
- `MODELRELAY_PUBLISHABLE_KEY` — publishable key for frontend token exchange.
- `MODELRELAY_BASE_URL` — override API base URL.
