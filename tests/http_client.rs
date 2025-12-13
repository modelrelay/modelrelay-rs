//! HTTP client tests using wiremock mock server.
//!
//! These tests verify:
//! - Request/response serialization for `POST /responses`
//! - Error handling
//! - Retry behavior
//! - Streaming (NDJSON) and structured streaming helpers

use futures_util::StreamExt;
use modelrelay::{Client, Config, Error, ResponseBuilder, RetryConfig};
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Clone)]
struct SequenceResponder {
    templates: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<ResponseTemplate>>>,
}

impl SequenceResponder {
    fn new(templates: Vec<ResponseTemplate>) -> Self {
        Self {
            templates: std::sync::Arc::new(std::sync::Mutex::new(templates.into_iter().collect())),
        }
    }
}

impl Respond for SequenceResponder {
    fn respond(&self, _req: &Request) -> ResponseTemplate {
        let mut templates = self.templates.lock().expect("mutex should not be poisoned");
        templates.pop_front().unwrap_or_else(|| {
            ResponseTemplate::new(500).set_body_json(json!({
                "error": { "message": "No more mock responses configured" }
            }))
        })
    }
}

/// Helper to create a client pointing at the mock server.
fn client_for_server(server: &MockServer) -> Client {
    Client::new(Config {
        api_key: Some("mr_sk_test_key".into()),
        base_url: Some(server.uri()),
        retry: Some(RetryConfig {
            max_attempts: 1,
            ..Default::default()
        }),
        ..Default::default()
    })
    .expect("client creation should succeed")
}

fn assistant_text(resp: &modelrelay::Response) -> String {
    let mut out = String::new();
    for item in &resp.output {
        let modelrelay::OutputItem::Message { role, content, .. } = item;
        if *role != modelrelay::MessageRole::Assistant {
            continue;
        }
        for part in content {
            let modelrelay::ContentPart::Text { text } = part;
            out.push_str(text);
        }
    }
    out
}

#[tokio::test]
async fn responses_sends_correct_request_and_parses_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header("X-ModelRelay-Api-Key", "mr_sk_test_key"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "model": "gpt-4o-mini",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "text", "text": "Hello!" }]
                }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_123",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Hello! How can I help you?" }]
                }
            ],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": { "input_tokens": 10, "output_tokens": 8, "total_tokens": 18 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Hello!")
        .send(&client.responses())
        .await
        .expect("request should succeed");

    assert_eq!(response.id, "resp_123");
    assert_eq!(assistant_text(&response), "Hello! How can I help you?");
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 8);
}

#[tokio::test]
async fn responses_includes_optional_parameters() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(body_json(json!({
            "model": "gpt-4o-mini",
            "input": [
                { "type": "message", "role": "system", "content": [{ "type": "text", "text": "You are helpful." }] },
                { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Hi" }] }
            ],
            "max_output_tokens": 100,
            "temperature": 0.7,
            "stop": ["END"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_456",
            "output": [
                { "type": "message", "role": "assistant", "content": [{ "type": "text", "text": "Hi there!" }] }
            ],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": { "input_tokens": 5, "output_tokens": 3, "total_tokens": 8 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .system("You are helpful.")
        .user("Hi")
        .max_output_tokens(100)
        .temperature(0.7)
        .stop(vec!["END".into()])
        .send(&client.responses())
        .await
        .expect("request should succeed");

    assert_eq!(response.id, "resp_456");
}

#[tokio::test]
async fn responses_handles_api_error_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "code": "INVALID_REQUEST", "message": "Invalid model specified" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let result = ResponseBuilder::new()
        .model("invalid-model")
        .user("Hello")
        .send(&client.responses())
        .await;

    match result {
        Err(Error::Api(api_err)) => {
            assert_eq!(api_err.status, 400);
            assert_eq!(api_err.code.as_deref(), Some("INVALID_REQUEST"));
            assert!(api_err.message.contains("Invalid model"));
        }
        other => panic!("expected API error, got {:?}", other),
    }
}

#[tokio::test]
async fn responses_retries_on_server_error() {
    let server = MockServer::start().await;

    // First request fails with 500; second succeeds.
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(SequenceResponder::new(vec![
            ResponseTemplate::new(500).set_body_json(json!({
                "error": { "message": "Server error" }
            })),
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_retry",
                "output": [
                    { "type": "message", "role": "assistant", "content": [{ "type": "text", "text": "ok" }] }
                ],
                "model": "gpt-4o-mini",
                "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 }
            })),
        ]))
        .expect(2)
        .mount(&server)
        .await;

    let client = Client::new(Config {
        api_key: Some("mr_sk_test_key".into()),
        base_url: Some(server.uri()),
        retry: Some(RetryConfig {
            max_attempts: 2,
            base_backoff: std::time::Duration::from_millis(0),
            max_backoff: std::time::Duration::from_millis(1),
            retry_post: true,
        }),
        ..Default::default()
    })
    .expect("client creation should succeed");

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("retry")
        .send(&client.responses())
        .await
        .expect("request should succeed after retry");

    assert_eq!(response.id, "resp_retry");
}

#[tokio::test]
async fn responses_streams_ndjson_events() {
    let server = MockServer::start().await;

    let ndjson = [
        json!({ "type": "start", "request_id": "resp_stream", "model": "gpt-4o-mini" }).to_string(),
        json!({ "type": "update", "payload": { "content": "hi" } }).to_string(),
        json!({
            "type": "completion",
            "payload": { "content": "hi" },
            "stop_reason": "stop",
            "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 }
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(ndjson),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let mut stream = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("hi")
        .stream(&client.responses())
        .await
        .expect("stream should succeed");

    let mut kinds = Vec::new();
    while let Some(evt) = stream.next().await {
        kinds.push(evt.expect("event").kind);
    }

    assert!(kinds.contains(&modelrelay::StreamEventKind::MessageStart));
    assert!(kinds.contains(&modelrelay::StreamEventKind::MessageDelta));
    assert!(kinds.contains(&modelrelay::StreamEventKind::MessageStop));
}

#[derive(Debug, Deserialize)]
struct Article {
    title: String,
}

#[tokio::test]
async fn responses_streams_structured_json() {
    let server = MockServer::start().await;

    let ndjson = [
        json!({ "type": "start", "request_id": "resp_structured" }).to_string(),
        json!({ "type": "update", "payload": { "title": "He" }, "complete_fields": [] }).to_string(),
        json!({ "type": "completion", "payload": { "title": "Hello" }, "complete_fields": ["title"] }).to_string(),
    ]
    .join("\n")
        + "\n";

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(ndjson),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let output_format = modelrelay::OutputFormat::json_schema(
        "Article",
        serde_json::json!({ "type": "object", "properties": { "title": { "type": "string" } } }),
    );

    let mut stream = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("write an article title")
        .output_format(output_format)
        .stream_json::<Article>(&client.responses())
        .await
        .expect("structured stream should succeed");

    let mut last: Option<Article> = None;
    while let Some(evt) = stream.next().await.expect("next") {
        last = Some(evt.payload);
        if evt.kind == modelrelay::StructuredRecordKind::Completion {
            break;
        }
    }

    assert_eq!(last.unwrap().title, "Hello");
}
