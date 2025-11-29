# Auth & API keys

The Rust SDK now targets server-side and trusted environments. Use a **secret API key** or a backend-issued bearer token; publishable-key frontend tokens have been removed.

## API key listing (server-side)

```rust
use modelrelay::{Client, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let keys = client.api_keys().list().await?;
    for key in keys {
        println!("{} ({})", key.redacted_key, key.kind);
    }
    Ok(())
}
```
