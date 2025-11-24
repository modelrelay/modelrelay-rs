# ModelRelay Rust SDK

Async and blocking clients for the ModelRelay API with optional SSE streaming. Default
features enable the async `reqwest` client and streaming; use the blocking feature when
you want to avoid Tokio in CLI/desktop tools.

## Installation

```toml
[dependencies]
# crates.io (after the first publish)
modelrelay = "0.1.0"

# Blocking-only (no Tokio/async runtime):
# modelrelay = { version = "0.1.0", default-features = false, features = ["blocking"] }

# Async without streaming:
# modelrelay = { version = "0.1.0", default-features = false, features = ["client"] }

# Local development / git fallback:
# modelrelay = { git = "https://github.com/modelrelay/modelrelay", package = "modelrelay" }
# modelrelay = { path = "sdk/rust" }
```

Features:

- `client` (default): async HTTP client built on `reqwest` + `tokio`.
- `blocking`: blocking HTTP client with `reqwest::blocking` (no Tokio runtime required).
- `streaming` (default): SSE streaming support for `/llm/proxy` (adds `reqwest/stream` + `futures`).

## Blocking LLM proxy (no Tokio)

```rust
use modelrelay::{
    BlockingClient, BlockingConfig, ProxyMessage, ProxyOptions, ProxyRequest,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest {
        model: "openai/gpt-4o-mini".into(),
        max_tokens: Some(128),
        stop_sequences: Some(vec!["```".into(), "</code>".into()]),
        messages: vec![ProxyMessage {
            role: "user".into(),
            content: "Write a short greeting without code fences.".into(),
        }],
        ..Default::default()
    };

    let completion = client
        .llm()
        .proxy(request, ProxyOptions::default().with_request_id("chat-123"))?;

    println!(
        "response {}: {} (stop={:?}, total_tokens={})",
        completion.id,
        completion.content.join(""),
        completion.stop_reason,
        completion.usage.total_tokens
    );
    Ok(())
}
```

`ProxyOptions` lets you set request IDs or extra headers:

```rust
let opts = ProxyOptions::default()
    .with_request_id("chat-123")
    .with_header("X-Debug", "true")
    .with_timeout(std::time::Duration::from_secs(30)); // per-call override
```

### Timeouts & retries

- Defaults: connect timeout 5s, request timeout 60s (non-streaming calls), 3 attempts with exponential backoff + jitter. Streaming calls leave the timeout unset unless you override it.
- Configure defaults on the client:

```rust
let client = Client::new(Config {
    api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
    timeout: Some(std::time::Duration::from_secs(30)),
    retry: Some(modelrelay::RetryConfig {
        max_attempts: 5,
        retry_post: true,
        ..Default::default()
    }),
    ..Default::default()
})?;
```

- Override per call (async or blocking):

```rust
let opts = ProxyOptions::default()
    .with_timeout(std::time::Duration::from_secs(10))
    .with_retry(modelrelay::RetryConfig {
        max_attempts: 1, // effectively disable retries
        ..Default::default()
    });
```

`RetryConfig::disabled()` is a helper to turn retries off, and `retry_post` controls whether POST
requests (like `/llm/proxy`) are retried.
```

## Ergonomic chat builders

You can build and send chat requests without hand-assembling `ProxyRequest`/`ProxyOptions`.

**Async non-streaming**

```rust
use modelrelay::{ChatRequestBuilder, Client, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let completion = ChatRequestBuilder::new("openai/gpt-4o")
        .message("user", "Summarize the Rust ownership model in 2 sentences.")
        .stop_sequences(vec!["```".into()])
        .request_id("chat-async-1")
        .send(&client.llm())
        .await?;

    println!("reply: {}", completion.content.join(""));
    Ok(())
}
```

**Async streaming with adapter**

```rust
use modelrelay::{ChatRequestBuilder, ChatStreamAdapter, Client, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let stream = ChatRequestBuilder::new("openai/gpt-4o-mini")
        .message("user", "Stream a 2-line Rust haiku.")
        .request_id("chat-stream-1")
        .stream(&client.llm())
        .await?;

    let mut adapter = ChatStreamAdapter::new(stream);
    while let Some(delta) = adapter.next_delta().await? {
        print!("{delta}");
    }

    if let Some(usage) = adapter.final_usage() {
        eprintln!("\nstop={:?} tokens={}", adapter.final_stop_reason(), usage.total_tokens);
    }
    Ok(())
}
```

**Blocking (non-streaming)**

```rust
use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let completion = ChatRequestBuilder::new("openai/gpt-4o-mini")
        .message("user", "Give me one fun Rust fact.")
        .request_id("chat-blocking-1")
        .send_blocking(&client.llm())?;

    println!("reply: {}", completion.content.join(""));
    Ok(())
}
```

**Blocking streaming with adapter**

```rust
use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder, ChatStreamAdapter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let stream = ChatRequestBuilder::new("openai/gpt-4o-mini")
        .message("user", "Stream a 2-line poem about ferris the crab.")
        .request_id("chat-blocking-stream-1")
        .stream_blocking(&client.llm())?;

    let mut adapter = ChatStreamAdapter::new(stream);
    while let Some(delta) = adapter.next_delta()? {
        print!("{delta}");
    }
    if let Some(usage) = adapter.final_usage() {
        eprintln!("\nstop={:?} tokens={}", adapter.final_stop_reason(), usage.total_tokens);
    }
    Ok(())
}
```

## Async LLM proxy (non-streaming)

```rust
use modelrelay::{Client, Config, ProxyMessage, ProxyRequest, ProxyOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest {
        model: "openai/gpt-4o".into(),
        max_tokens: Some(128),
        messages: vec![ProxyMessage {
            role: "user".into(),
            content: "Write a short greeting without code fences.".into(),
        }],
        ..Default::default()
    };

    let completion = client.llm().proxy(request, ProxyOptions::default()).await?;
    println!(
        "response {}: {} (stop={:?}, total_tokens={})",
        completion.id,
        completion.content.join(""),
        completion.stop_reason,
        completion.usage.total_tokens
    );
    Ok(())
}
```

## Frontend token exchange (publishable key flow)

```rust
use modelrelay::{Client, Config, FrontendTokenRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use a publishable key (mr_pk_...) to mint a short-lived bearer token for browsers/mobile.
    let auth = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_PUBLISHABLE_KEY")?),
        ..Default::default()
    })?;

    let token = auth
        .auth()
        .frontend_token(FrontendTokenRequest {
            user_id: "user-123".into(),
            device_id: Some("device-abc".into()),
            ..Default::default()
        })
        .await?;

    // Use the frontend token directly with another client.
    let client = Client::new(Config {
        access_token: Some(token.token.clone()),
        ..Default::default()
    })?;
    // ... call client.llm().proxy(...) or proxy_stream(...) with end-user context
    Ok(())
}
```

## API key management (server-side)

```rust
use modelrelay::{APIKeyCreateRequest, Client, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let created = client
        .api_keys()
        .create(APIKeyCreateRequest {
            label: "rust-sdk-demo".into(),
            ..Default::default()
        })
        .await?;
    println!("created key {}", created.redacted_key);

    let keys = client.api_keys().list().await?;
    for key in keys {
        println!("{} ({})", key.redacted_key, key.kind);
    }
    Ok(())
}
```

## Errors

- `Error::Config` — missing/invalid base URL, key/token, or bad headers.
- `Error::Transport` — timeouts, DNS/TLS/connectivity (inspect `TransportErrorKind` and `retries`).
- `Error::Api` — structured API errors (`APIError` includes status/code/fields/request_id and retry metadata).
- `Error::Serialization` — JSON parsing/decoding failures.

## Publishing

- Verify packaging before release: `cd sdk/rust && cargo package` (or `cargo publish --dry-run`).
- Publish + subtree sync + signed tag on the public repo: `just sdk-release-rust 0.1.0` (needs crates.io token, signing key, and access to `git@github.com:modelrelay/modelrelay-rs.git`).
- Manual steps if you prefer: `cargo publish`, `git subtree push --prefix sdk/rust git@github.com:modelrelay/modelrelay-rs.git main`, `git fetch git@github.com:modelrelay/modelrelay-rs.git main:modelrelay-rs-main`, `git tag -s v0.1.0 modelrelay-rs-main -m "modelrelay v0.1.0"`, `git push git@github.com:modelrelay/modelrelay-rs.git v0.1.0`.

## Environment variables

- `MODELRELAY_API_KEY` — secret key for server-to-server calls.
- `MODELRELAY_PUBLISHABLE_KEY` — publishable key for frontend token exchange.
- `MODELRELAY_BASE_URL` — override API base URL (defaults to `https://api.modelrelay.ai/api/v1`).
