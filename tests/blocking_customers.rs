//! Blocking client tests for customer bearer token endpoints.

#![cfg(feature = "blocking")]

use modelrelay::{generated::PriceInterval, BlockingClient, BlockingConfig};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn blocking_customers_me_subscription_sends_request_and_parses_response() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime should start");

    let server = rt.block_on(async { MockServer::start().await });

    rt.block_on(async {
        Mock::given(method("GET"))
            .and(path("/customers/me/subscription"))
            .and(header("authorization", "Bearer header.payload.signature"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "subscription": {
                    "tier_code": "pro",
                    "tier_display_name": "Pro",
                    "price_amount_cents": 2000,
                    "price_currency": "usd",
                    "price_interval": "month",
                    "subscription_status": "active",
                    "current_period_start": "2025-01-01T00:00:00Z",
                    "current_period_end": "2025-02-01T00:00:00Z"
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
        .me_subscription()
        .expect("me_subscription request should succeed");

    assert_eq!(usage.tier_code.to_string(), "pro");
    assert_eq!(usage.price_amount_cents, Some(2000));
    assert_eq!(usage.price_currency.as_deref(), Some("usd"));
    assert_eq!(usage.price_interval, Some(PriceInterval::Month));
}

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
                    "requests": 3,
                    "tokens": 150,
                    "daily": [
                        { "day": "2025-01-01T00:00:00Z", "requests": 1, "tokens": 50 },
                        { "day": "2025-01-02T00:00:00Z", "requests": 2, "tokens": 100 }
                    ]
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

    assert_eq!(usage.requests, 3);
    assert_eq!(usage.tokens, 150);
    assert_eq!(usage.daily.len(), 2);
}
