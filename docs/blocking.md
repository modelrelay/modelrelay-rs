# Blocking usage

## Non-streaming

```rust
use modelrelay::{BlockingClient, BlockingConfig, Model, ProxyMessage, ProxyOptions, ProxyRequest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let request = ProxyRequest::new(
        Model::OpenAIGpt4oMini,
        vec![ProxyMessage {
            role: "user".into(),
            content: "Write a short greeting.".into(),
        }],
    )?;

    let completion = client
        .llm()
        .proxy(request, ProxyOptions::default().with_request_id("chat-blocking-1"))?;

    println!("response {}: {}", completion.id, completion.content.join(""));
    Ok(())
}
```

## Streaming

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
        eprintln!("\nstop={:?} tokens={}", adapter.final_stop_reason(), usage.total());
    }
    Ok(())
}
```
