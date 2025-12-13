# Tracing + metrics

Optional, off by default:
- Enable `tracing` feature for spans/events around HTTP + streaming.
- Use `MetricsCallbacks` to capture latency/usage without requiring tracing.

```toml
[dependencies]
modelrelay = { version = "0.45.0", features = ["tracing", "streaming"] }
tracing-subscriber = "0.3"
```

```rust
use std::sync::Arc;
use modelrelay::{
    Client, Config, HttpRequestMetrics, MetricsCallbacks, ResponseBuilder,
    StreamFirstTokenMetrics, TokenUsageMetrics,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("modelrelay=trace").init();

    let metrics = MetricsCallbacks {
        http_request: Some(Arc::new(|m: HttpRequestMetrics| {
            eprintln!(
                "[http] {} {} => {:?} ({:?}) req_id={:?}",
                m.context.method, m.context.path, m.status, m.latency, m.context.request_id
            );
        })),
        stream_first_token: Some(Arc::new(|m: StreamFirstTokenMetrics| {
            eprintln!(
                "[first-token] latency_ms={} req_id={:?} resp_id={:?}",
                m.latency.as_millis(),
                m.context.request_id,
                m.context.response_id
            );
        })),
        usage: Some(Arc::new(|m: TokenUsageMetrics| {
            eprintln!(
                "[usage] model={:?} total_tokens={}",
                m.context.model,
                m.usage.total()
            );
        })),
    };

    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        metrics: Some(metrics),
        ..Default::default()
    })?;

    let mut deltas = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Stream a sentence about telemetry.")
        .request_id("responses-metrics-async")
        .stream_deltas(&client.responses())
        .await?;

    use futures_util::StreamExt;
    while let Some(delta) = deltas.next().await {
        print!("{}", delta?);
    }
    Ok(())
}
```
