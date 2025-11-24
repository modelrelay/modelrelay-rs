# ModelRelay Rust SDK

Async client for the ModelRelay API with blocking and streaming LLM proxy helpers.
By default the crate enables the HTTP client and SSE streaming; you can turn features
off to avoid pulling in `tokio`/`reqwest` when you only need the types.

## Installation

```toml
[dependencies]
modelrelay = { git = "https://github.com/modelrelay/modelrelay", package = "modelrelay" }
# When working from a local clone of this repo:
# modelrelay = { path = "sdk/rust" }
# Disable streaming to skip the reqwest `stream` feature:
# modelrelay = { path = "../../sdk/rust", default-features = false, features = ["client"] }
```

Features:

- `client` (default): enables the HTTP client built on `reqwest` + `tokio`.
- `streaming` (default): SSE streaming support for `/llm/proxy` (adds `reqwest/stream` + `futures`).

## Blocking LLM proxy

```rust
use modelrelay::{Client, Config, ProxyMessage, ProxyRequest, ProxyOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    // Use stop sequences to suppress fenced responses.
    let request = ProxyRequest {
        model: "openai/gpt-4o".into(),
        max_tokens: Some(128),
        stop_sequences: Some(vec!["```".into(), "</code>".into()]),
        messages: vec![ProxyMessage {
            role: "user".into(),
            content: "Write a short greeting without code fences.".into(),
        }],
        ..Default::default()
    };

    let completion = client.llm().proxy(request, ProxyOptions::default()).await?;
    println!(
        "response {} (stop={}): {}",
        completion.id,
        completion.stop_reason.unwrap_or_default(),
        completion.content.join("")
    );
    Ok(())
}
```

`ProxyOptions` lets you set request IDs or extra headers:

```rust
let opts = ProxyOptions::default().with_request_id("chat-123");
```

## Streaming SSE

```rust
use futures_util::StreamExt;
use modelrelay::{Client, Config, ProxyMessage, ProxyRequest, ProxyOptions, StreamEventKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest {
        model: "openai/gpt-4o".into(),
        messages: vec![ProxyMessage {
            role: "user".into(),
            content: "Stream a haiku about Rust.".into(),
        }],
        ..Default::default()
    };

    let mut stream = client.llm().proxy_stream(request, ProxyOptions::default()).await?;
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
                    eprintln!(" stop_reason={:?} total={}", event.stop_reason, usage.total_tokens);
                }
            }
            _ => {}
        }
    }

    // If you need to abort mid-stream:
    // stream.cancel();
    Ok(())
}
```

Each `StreamEvent` exposes the raw payload, parsed text delta, response ID, stop reason,
usage, and the request ID echoed from the headers.

## Auth helpers

- `client.auth().frontend_token(...)` exchanges a publishable key + user/device identifiers
  for a short-lived bearer token suitable for browsers/mobile apps.
- `client.api_keys()` lists, creates, or deletes API keys for the authenticated project.
