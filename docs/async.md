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

    let completion = ChatRequestBuilder::new("openai/gpt-4o-mini")
        .message("user", "Summarize Rust ownership in 2 sentences.")
        .request_id("chat-async-1")
        .send(&client.llm())
        .await?;

    println!("reply: {}", completion.content.join(""));
    Ok(())
}
```

## Streaming

```rust
use modelrelay::{ChatRequestBuilder, ChatStreamAdapter, Client, Config};
use futures_util::StreamExt;

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

    let mut deltas = ChatStreamAdapter::new(stream).into_stream();
    while let Some(delta) = deltas.next().await {
        print!("{}", delta?);
    }

    Ok(())
}
```
