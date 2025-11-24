# Offline testing with mocks

Enable the `mock` feature for in-memory clients + fixtures (no network):

```toml
[dev-dependencies]
modelrelay = { path = "sdk/rust", features = ["mock", "streaming"] }
futures-util = "0.3" # for StreamExt in async tests
```

```rust
use futures_util::StreamExt;
use modelrelay::{
    fixtures, ChatRequestBuilder, MockClient, MockConfig, Model, ProxyMessage, ProxyOptions,
    ProxyRequest,
};

#[tokio::test]
async fn offline_completion_and_stream() -> Result<(), modelrelay::Error> {
    let client = MockClient::new(
        MockConfig::default()
            .with_proxy_response(fixtures::simple_proxy_response())
            .with_stream_events(fixtures::simple_stream_events()),
    );

    let completion = client
        .llm()
        .proxy(
            ProxyRequest::new(
                Model::OpenAIGpt4oMini,
                vec![ProxyMessage {
                    role: "user".into(),
                    content: "hi".into(),
                }],
            )?,
            ProxyOptions::default(),
        )
        .await?;
    assert_eq!(completion.content.join(""), "hello world");

    let mut stream = ChatRequestBuilder::new("openai/gpt-4o-mini")
        .message("user", "stream me something")
        .stream(&client.llm())
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

Blocking mocks are available via `MockClient::blocking_llm()`; streaming/non-streaming helpers work the same way.
