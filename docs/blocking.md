# Blocking usage

Enable the blocking client when you do not want to pull in Tokio (perfect for small CLIs):

```toml
[dependencies]
modelrelay = { version = "0.3.3", default-features = false, features = ["blocking"] }
# add streaming support without Tokio:
# modelrelay = { version = "0.3.3", default-features = false, features = ["blocking", "streaming"] }
```

## Non-streaming

```rust
use modelrelay::{BlockingClient, BlockingConfig, Model, ProxyOptions, ProxyRequest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest::builder(Model::Gpt4oMini)
        .user("Write a short greeting.")
        .build()?;

    let completion = client
        .llm()
        .proxy(request, ProxyOptions::default().with_request_id("chat-blocking-1"))?;

    println!("response {}: {}", completion.id, completion.content.join(""));
    Ok(())
}
```

## Streaming

```rust
use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let iter = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Stream a 2-line poem about ferris the crab.")
        .request_id("chat-blocking-stream-1")
        .stream_deltas_blocking(&client.llm())?;

    for delta in iter {
        print!("{}", delta?);
    }
    Ok(())
}
```
