use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::{ApiKey, Client, Config, RetryConfig};

/// Create a test client configured to use a wiremock server.
/// Disables retries by default for predictable test behavior.
pub fn test_client(base_url: &str) -> Client {
    Client::new(Config {
        base_url: Some(base_url.to_string()),
        api_key: Some(ApiKey::parse("mr_sk_test").expect("api key")),
        retry: Some(RetryConfig::disabled()),
        ..Default::default()
    })
    .expect("client")
}

/// Create a test client with a publishable key.
pub fn test_client_publishable(base_url: &str) -> Client {
    Client::new(Config {
        base_url: Some(base_url.to_string()),
        api_key: Some(ApiKey::parse("mr_pk_test").expect("api key")),
        retry: Some(RetryConfig::disabled()),
        ..Default::default()
    })
    .expect("client")
}

/// Start a local NDJSON server that streams chunked responses.
pub async fn start_chunked_ndjson_server(
    steps: Vec<(Duration, String)>,
    finish_after: Option<Duration>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    tokio::spawn(async move {
        let (mut socket, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => return,
        };

        // Read request headers.
        let mut buf = [0u8; 4096];
        let mut received = Vec::new();
        loop {
            let n = match socket.read(&mut buf).await {
                Ok(n) => n,
                Err(_) => return,
            };
            if n == 0 {
                return;
            }
            received.extend_from_slice(&buf[..n]);
            if received.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let headers = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: application/x-ndjson\r\n",
            "Transfer-Encoding: chunked\r\n",
            "\r\n"
        );
        if socket.write_all(headers.as_bytes()).await.is_err() {
            return;
        }

        for (delay, line) in steps {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let payload = format!("{line}\n");
            let chunk = format!("{:X}\r\n{}\r\n", payload.len(), payload);
            if socket.write_all(chunk.as_bytes()).await.is_err() {
                return;
            }
        }

        if let Some(delay) = finish_after {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
        }

        let _ = socket.write_all(b"0\r\n\r\n").await;
    });

    format!("http://{}", addr)
}

/// Start a local NDJSON server that immediately emits all lines.
pub async fn start_ndjson_server(lines: Vec<String>) -> String {
    let steps = lines
        .into_iter()
        .map(|line| (Duration::from_millis(0), line))
        .collect();
    start_chunked_ndjson_server(steps, None).await
}
