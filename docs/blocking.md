# Blocking usage

Enable the blocking client when you do not want to pull in Tokio (perfect for small CLIs):

```toml
[dependencies]
modelrelay = { version = "0.46.2", default-features = false, features = ["blocking"] }
# add streaming support without Tokio:
# modelrelay = { version = "0.46.2", default-features = false, features = ["blocking", "streaming"] }
```

## Non-streaming

```rust
use modelrelay::{BlockingClient, BlockingConfig, ResponseBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Write a short greeting.")
        .request_id("blocking-1")
        .send_blocking(&client.responses())?;

    println!("response {}: {:?}", response.id, response.output);
    Ok(())
}
```

## Streaming

```rust
use modelrelay::{BlockingClient, BlockingConfig, ResponseBuilder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let iter = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Stream a 2-line poem about ferris the crab.")
        .request_id("blocking-stream-1")
        .stream_deltas_blocking(&client.responses())?;

    for delta in iter {
        print!("{}", delta?);
    }
    Ok(())
}
```
