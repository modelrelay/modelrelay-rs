# Async usage

## Non-streaming

```rust
use modelrelay::{Client, Config, ResponseBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Summarize Rust ownership in 2 sentences.")
        .request_id("responses-async-1")
        .send(&client.responses())
        .await?;

    let mut text = String::new();
    for item in &response.output {
        let modelrelay::OutputItem::Message { role, content, .. } = item;
        if *role != modelrelay::MessageRole::Assistant {
            continue;
        }
        for part in content {
            if let modelrelay::ContentPart::Text { text: t } = part {
                text.push_str(t);
            }
        }
    }
    println!("reply: {}", text);
    Ok(())
}
```

## Streaming

```rust
use modelrelay::{Client, Config, ResponseBuilder};
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let mut deltas = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Stream a 2-line Rust haiku.")
        .request_id("responses-stream-1")
        .stream_deltas(&client.responses())
        .await?;
    futures_util::pin_mut!(deltas);
    while let Some(delta) = deltas.next().await {
        print!("{}", delta?);
    }

    Ok(())
}
```

> Need request IDs or usage? Use `.stream(&client.responses())` and inspect the NDJSON envelopes or call `.collect()` for the aggregated response.
