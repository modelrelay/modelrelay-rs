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
    .with_header("X-Debug", "true");
```

## Async streaming quick start

```rust
use futures_util::StreamExt;
use modelrelay::{
    Client, Config, ProxyMessage, ProxyOptions, ProxyRequest, StreamEventKind,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest {
        model: "openai/gpt-4o".into(),
        stop_sequences: Some(vec!["```".into()]), // fence suppression
        messages: vec![ProxyMessage {
            role: "user".into(),
            content: "Stream a 2-line poem about Rust.".into(),
        }],
        ..Default::default()
    };

    let mut stream = client
        .llm()
        .proxy_stream(request, ProxyOptions::default().with_request_id("chat-42"))
        .await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        match event.kind {
            StreamEventKind::MessageDelta => {
                if let Some(delta) = event.text_delta {
                    print!("{delta}");
                }
            }
            StreamEventKind::MessageStop => {
                if let Some(usage) = event.usage {
                    eprintln!(
                        "\nstop_reason={:?} total_tokens={}",
                        event.stop_reason, usage.total_tokens
                    );
                }
            }
            _ => {}
        }
    }

    // Abort mid-stream if needed:
    // stream.cancel();
    Ok(())
}
```

`StreamEvent` includes the raw payload, parsed text delta, response ID, stop reason, usage,
and the echoed request ID.

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

## Publishing

- Verify packaging before release: `cd sdk/rust && cargo package` (or `cargo publish --dry-run`).
- Publish + subtree sync + signed tag on the public repo: `just sdk-release-rust 0.1.0` (needs crates.io token, signing key, and access to `git@github.com:modelrelay/modelrelay-rs.git`).
- Manual steps if you prefer: `cargo publish`, `git subtree push --prefix sdk/rust git@github.com:modelrelay/modelrelay-rs.git main`, `git fetch git@github.com:modelrelay/modelrelay-rs.git main:modelrelay-rs-main`, `git tag -s v0.1.0 modelrelay-rs-main -m "modelrelay v0.1.0"`, `git push git@github.com:modelrelay/modelrelay-rs.git v0.1.0`.

## Environment variables

- `MODELRELAY_API_KEY` — secret key for server-to-server calls.
- `MODELRELAY_PUBLISHABLE_KEY` — publishable key for frontend token exchange.
- `MODELRELAY_BASE_URL` — override API base URL (defaults to `https://api.modelrelay.ai/api/v1`).
