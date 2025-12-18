//! Blocking client tests for customer bearer token endpoints.

#![cfg(feature = "blocking")]

use modelrelay::{BlockingClient, BlockingConfig};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn blocking_customers_me_usage_sends_request_and_parses_response() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime should start");

    let server = rt.block_on(async { MockServer::start().await });

    rt.block_on(async {
        Mock::given(method("GET"))
            .and(path("/customers/me/usage"))
            .and(header("authorization", "Bearer header.payload.signature"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "usage": {
                    "window_start": "2025-01-01T00:00:00Z",
                    "window_end": "2025-02-01T00:00:00Z",
                    "spend_limit_cents": 1000,
                    "current_spend_cents": 250,
                    "remaining_cents": 750,
                    "percentage_used": 25.0,
                    "state": "allowed"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
    });

    let client = BlockingClient::new(BlockingConfig {
        base_url: Some(server.uri()),
        access_token: Some("header.payload.signature".to_string()),
        ..Default::default()
    })
    .expect("client creation should succeed");

    let usage = client
        .customers()
        .me_usage()
        .expect("me_usage request should succeed");

    assert_eq!(usage.spend_limit_cents, 1000);
    assert_eq!(usage.current_spend_cents, 250);
    assert_eq!(usage.remaining_cents, 750);
}
