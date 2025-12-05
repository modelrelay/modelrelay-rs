# Async usage

## Non-streaming

```rust
use modelrelay::{ChatRequestBuilder, Client, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let completion = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Summarize Rust ownership in 2 sentences.")
        .request_id("chat-async-1")
        .send(&client.llm())
        .await?;

    println!("reply: {}", completion.content.join(""));
    Ok(())
}
```

## Streaming

```rust
use modelrelay::{ChatRequestBuilder, Client, Config};
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let mut deltas = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Stream a 2-line Rust haiku.")
        .request_id("chat-stream-1")
        .stream_deltas(&client.llm())
        .await?;
    futures_util::pin_mut!(deltas);
    while let Some(delta) = deltas.next().await {
        print!("{}", delta?);
    }

    Ok(())
}
```

> Need request IDs or usage? Use `proxy_stream` + `ChatStreamAdapter` directly to access `request_id`, `final_usage`, or `final_stop_reason`.
