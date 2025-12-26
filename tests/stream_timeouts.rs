use std::time::Duration;

use futures_util::StreamExt;
use modelrelay::{
    testing::start_chunked_ndjson_server, ApiKey, Client, Config, Error, ResponseBuilder,
    StreamTimeoutKind,
};

#[tokio::test]
async fn stream_ttft_timeout() {
    let base_url = start_chunked_ndjson_server(vec![], Some(Duration::from_millis(200))).await;

    let client = Client::new(Config {
        base_url: Some(base_url),
        api_key: Some(ApiKey::parse("mr_sk_test").unwrap()),
        retry: Some(modelrelay::RetryConfig::disabled()),
        ..Default::default()
    })
    .expect("client");

    let mut stream = ResponseBuilder::new()
        .model("gpt-4o")
        .user("hi")
        .stream_ttft_timeout(Duration::from_millis(1))
        .stream(&client.responses())
        .await
        .expect("stream handle");

    let err = stream
        .next()
        .await
        .expect("stream result")
        .expect_err("timeout");
    match err {
        Error::StreamTimeout(te) => assert_eq!(te.kind, StreamTimeoutKind::Ttft),
        other => panic!("expected stream timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn stream_idle_timeout() {
    let base_url = start_chunked_ndjson_server(
        vec![(
            Duration::from_millis(0),
            "{\"type\":\"update\",\"delta\":\"hi\"}".to_string(),
        )],
        Some(Duration::from_millis(200)),
    )
    .await;

    let client = Client::new(Config {
        base_url: Some(base_url),
        api_key: Some(ApiKey::parse("mr_sk_test").unwrap()),
        retry: Some(modelrelay::RetryConfig::disabled()),
        ..Default::default()
    })
    .expect("client");

    let mut stream = ResponseBuilder::new()
        .model("gpt-4o")
        .user("hi")
        .stream_idle_timeout(Duration::from_millis(5))
        .stream(&client.responses())
        .await
        .expect("stream handle");

    let first = stream.next().await.expect("first event").expect("ok event");
    assert!(first.text_delta.is_some());

    let err = stream
        .next()
        .await
        .expect("stream result")
        .expect_err("timeout");
    match err {
        Error::StreamTimeout(te) => assert_eq!(te.kind, StreamTimeoutKind::Idle),
        other => panic!("expected idle timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn stream_total_timeout() {
    let mut steps = Vec::new();
    steps.push((
        Duration::from_millis(0),
        "{\"type\":\"update\",\"delta\":\"hi\"}".to_string(),
    ));
    for _ in 0..10 {
        steps.push((
            Duration::from_millis(10),
            "{\"type\":\"keepalive\"}".to_string(),
        ));
    }
    let base_url = start_chunked_ndjson_server(steps, Some(Duration::from_millis(200))).await;

    let client = Client::new(Config {
        base_url: Some(base_url),
        api_key: Some(ApiKey::parse("mr_sk_test").unwrap()),
        retry: Some(modelrelay::RetryConfig::disabled()),
        ..Default::default()
    })
    .expect("client");

    let mut stream = ResponseBuilder::new()
        .model("gpt-4o")
        .user("hi")
        .stream_total_timeout(Duration::from_millis(25))
        .stream(&client.responses())
        .await
        .expect("stream handle");

    let first = stream.next().await.expect("first event").expect("ok event");
    assert!(first.text_delta.is_some());

    let err = stream
        .next()
        .await
        .expect("stream result")
        .expect_err("timeout");
    match err {
        Error::StreamTimeout(te) => assert_eq!(te.kind, StreamTimeoutKind::Total),
        other => panic!("expected total timeout, got {other:?}"),
    }
}
