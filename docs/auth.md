# Auth & API keys

## Frontend token exchange (publishable key flow)

```rust
use modelrelay::{Client, Config, FrontendTokenRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let auth = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_PUBLISHABLE_KEY")?),
        ..Default::default()
    })?;

    let token = auth
        .auth()
        .frontend_token(FrontendTokenRequest {
            user_id: "user-123".into(),
            device_id: Some("device-abc".into()),
            ..Default::default()
        })
        .await?;

    let client = Client::new(Config {
        access_token: Some(token.token.clone()),
        ..Default::default()
    })?;
    // ... call client.llm().proxy(...) or proxy_stream(...)
    Ok(())
}
```

## API key management (server-side)

```rust
use modelrelay::{APIKeyCreateRequest, Client, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config {
        api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
        ..Default::default()
    })?;

    let created = client
        .api_keys()
        .create(APIKeyCreateRequest {
            label: "rust-sdk-demo".into(),
            ..Default::default()
        })
        .await?;
    println!("created key {}", created.redacted_key);

    let keys = client.api_keys().list().await?;
    for key in keys {
        println!("{} ({})", key.redacted_key, key.kind);
    }
    Ok(())
}
```
