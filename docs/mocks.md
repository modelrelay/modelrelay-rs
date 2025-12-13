# Offline testing with mocks

Enable the `mock` feature for in-memory clients + fixtures (no network):

```toml
[dev-dependencies]
modelrelay = { path = "sdk/rust", features = ["mock", "streaming"] }
futures-util = "0.3" # for StreamExt in async tests
```

```rust
use futures_util::StreamExt;
use modelrelay::{fixtures, MockClient, MockConfig, ResponseBuilder};

#[tokio::test]
async fn offline_completion_and_stream() -> Result<(), modelrelay::Error> {
    let client = MockClient::new(
        MockConfig::default()
            .with_response(fixtures::simple_response())
            .with_stream_events(fixtures::simple_stream_events()),
    );

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("hi")
        .send(&client.responses())
        .await?;
    let mut text = String::new();
    for item in &response.output {
        let modelrelay::OutputItem::Message { role, content, .. } = item;
        if *role != modelrelay::MessageRole::Assistant {
            continue;
        }
        for part in content {
            let modelrelay::ContentPart::Text { text: t } = part;
            text.push_str(t);
        }
    }
    assert_eq!(text, "hello world");

    let mut stream = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("stream me something")
        .stream(&client.responses())
        .await?;

    let mut text = String::new();
    while let Some(evt) = stream.next().await {
        let evt = evt?;
        if let Some(delta) = evt.text_delta {
            text.push_str(&delta);
        }
    }
    assert_eq!(text, "hello");
    Ok(())
}
```

Blocking mocks are available via `MockClient::blocking_responses()`; streaming/non-streaming helpers work the same way.
