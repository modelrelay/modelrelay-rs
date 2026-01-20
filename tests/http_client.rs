//! HTTP client tests using wiremock mock server.
//!
//! These tests verify:
//! - Request/response serialization for `POST /responses`
//! - Error handling
//! - Retry behavior
//! - Streaming (NDJSON) and structured streaming helpers

use std::num::NonZero;

use futures_util::StreamExt;
use modelrelay::{
    testing::start_chunked_ndjson_server, ApiKey, Client, Config, Error, ListStateHandlesOptions,
    NodeId, RequestId, ResponseBuilder, RetryConfig, RunId, RunsToolCallV0, RunsToolResultItemV0,
    RunsToolResultsRequest, StateHandleCreateRequest, ToolCallId, ToolName,
    MAX_STATE_HANDLE_TTL_SECONDS,
};
use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param};
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
        api_key: Some(ApiKey::parse("mr_sk_test_key").unwrap()),
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
            if let modelrelay::ContentPart::Text { text } = part {
                out.push_str(text);
            }
        }
    }
    out
}

#[tokio::test]
async fn runs_submit_tool_results_posts_payload() {
    let server = MockServer::start().await;

    let run_id: RunId = "550e8400-e29b-41d4-a716-446655440000"
        .parse()
        .expect("run id");
    let node_id: NodeId = "worker".parse().expect("node id");
    let request_id: RequestId = "550e8400-e29b-41d4-a716-446655440001"
        .parse()
        .expect("request id");
    let tool_call_id: ToolCallId = "call_1".parse().expect("tool call id");
    let tool_name: ToolName = "echo".parse().expect("tool name");

    Mock::given(method("POST"))
        .and(path(format!("/runs/{}/tool-results", run_id)))
        .and(body_json(json!({
            "node_id": "worker",
            "step": 2,
            "request_id": request_id.clone(),
            "results": [{
                "tool_call": {
                    "id": tool_call_id.clone(),
                    "name": tool_name.clone()
                },
                "output": "ok"
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "accepted": 1,
            "status": "waiting"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);
    let resp = client
        .runs()
        .submit_tool_results(
            run_id,
            RunsToolResultsRequest {
                node_id,
                step: 2,
                request_id,
                results: vec![RunsToolResultItemV0 {
                    tool_call: RunsToolCallV0 {
                        id: tool_call_id,
                        name: tool_name,
                        arguments: None,
                    },
                    output: "ok".to_string(),
                }],
            },
        )
        .await
        .expect("request should succeed");

    assert_eq!(resp.accepted, 1);
}

#[tokio::test]
async fn responses_text_happy_path() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(body_json(json!({
            "model": "gpt-4o-mini",
            "input": [
                { "type": "message", "role": "system", "content": [{ "type": "text", "text": "Be concise." }] },
                { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Hello" }] }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_text_1",
            "output": [
                { "type": "message", "role": "assistant", "content": [{ "type": "text", "text": "Hi!" }] }
            ],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": { "input_tokens": 3, "output_tokens": 1, "total_tokens": 4 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);
    let text = client
        .responses()
        .text("gpt-4o-mini", "Be concise.", "Hello")
        .await
        .expect("request should succeed");
    assert_eq!(text, "Hi!");
}

#[tokio::test]
async fn state_handles_create_posts_payload() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/state-handles"))
        .and(body_json(json!({ "ttl_seconds": 3600 })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "project_id": "11111111-2222-3333-4444-555555555555",
            "created_at": "2025-01-15T10:30:00.000Z",
            "expires_at": "2025-01-15T11:30:00.000Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);
    let resp = client
        .state_handles()
        .create(StateHandleCreateRequest {
            ttl_seconds: Some(NonZero::new(3600).unwrap()),
        })
        .await
        .expect("create should succeed");

    assert_eq!(resp.id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
}

#[tokio::test]
async fn state_handles_create_rejects_large_ttl() {
    let server = MockServer::start().await;
    let client = client_for_server(&server);

    let ttl = std::num::NonZeroU64::new(MAX_STATE_HANDLE_TTL_SECONDS + 1).unwrap();
    let err = client
        .state_handles()
        .create(StateHandleCreateRequest {
            ttl_seconds: Some(ttl),
        })
        .await
        .expect_err("expected ttl validation error");

    match err {
        Error::Validation(err) => {
            assert!(err.message.contains("ttl_seconds exceeds maximum"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn state_handles_list_and_delete() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/state-handles"))
        .and(query_param("limit", "1"))
        .and(query_param("offset", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "state_handles": [
                {
                    "id": "550e8400-e29b-41d4-a716-446655440000",
                    "project_id": "11111111-2222-3333-4444-555555555555",
                    "created_at": "2025-01-15T10:30:00.000Z"
                }
            ],
            "next_cursor": "3"
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path("/state-handles/550e8400-e29b-41d4-a716-446655440000"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);
    let list = client
        .state_handles()
        .list(ListStateHandlesOptions {
            limit: Some(1),
            offset: Some(2),
        })
        .await
        .expect("list should succeed");
    assert_eq!(list.state_handles.len(), 1);

    client
        .state_handles()
        .delete("550e8400-e29b-41d4-a716-446655440000")
        .await
        .expect("delete should succeed");
}

#[tokio::test]
async fn responses_text_errors_on_empty_output() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_text_empty",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "noop", "arguments": "{}" } }]
                }
            ],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);
    let err = client
        .responses()
        .text("gpt-4o-mini", "sys", "user")
        .await
        .expect_err("empty assistant text should error");
    match err {
        Error::Transport(te) => assert_eq!(te.kind, modelrelay::TransportErrorKind::EmptyResponse),
        other => panic!("expected transport error, got {:?}", other),
    }
}

#[tokio::test]
async fn responses_text_requires_model_without_customer_id() {
    let server = MockServer::start().await;
    let client = client_for_server(&server);

    let err = ResponseBuilder::text_prompt("sys", "user")
        .send_text(&client.responses())
        .await
        .expect_err("missing model should fail validation");

    match err {
        Error::Validation(ve) => assert_eq!(ve.field.as_deref(), Some("model")),
        other => panic!("expected validation error, got {:?}", other),
    }

    let requests = server
        .received_requests()
        .await
        .expect("should be able to read received requests");
    assert!(
        requests.is_empty(),
        "request should not be sent on validation failure"
    );
}

#[tokio::test]
async fn responses_text_allows_customer_id_without_model() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header("X-ModelRelay-Customer-Id", "customer-123"))
        .and(body_json(json!({
            "input": [
                { "type": "message", "role": "system", "content": [{ "type": "text", "text": "sys" }] },
                { "type": "message", "role": "user", "content": [{ "type": "text", "text": "user" }] }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_customer",
            "output": [
                { "type": "message", "role": "assistant", "content": [{ "type": "text", "text": "ok" }] }
            ],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": { "input_tokens": 2, "output_tokens": 1, "total_tokens": 3 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);
    let text = client
        .responses()
        .text_for_customer("customer-123", "sys", "user")
        .await
        .expect("request should succeed");
    assert_eq!(text, "ok");
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
        api_key: Some(ApiKey::parse("mr_sk_test_key").unwrap()),
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
        json!({ "type": "update", "delta": "hi" }).to_string(),
        json!({
            "type": "completion",
            "stop_reason": "stop",
            "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 }
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header(
            "accept",
            "application/x-ndjson; profile=\"responses-stream/v2\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ndjson, "application/x-ndjson"))
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

#[tokio::test]
async fn responses_stream_deltas_emits_completion_only_content() {
    let server = MockServer::start().await;

    let ndjson = [
        json!({ "type": "start", "request_id": "resp_stream", "model": "gpt-4o-mini" }).to_string(),
        json!({
            "type": "completion",
            "content": "hi",
            "stop_reason": "stop",
            "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 }
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header(
            "accept",
            "application/x-ndjson; profile=\"responses-stream/v2\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ndjson, "application/x-ndjson"))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let mut deltas = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("hi")
        .stream_deltas(&client.responses())
        .await
        .expect("stream deltas should succeed");

    let mut out = Vec::new();
    while let Some(delta) = deltas.next().await {
        out.push(delta.expect("delta"));
    }

    assert_eq!(out, vec!["hi".to_string()]);
}

#[tokio::test]
async fn responses_stream_rejects_non_ndjson_content_type() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header(
            "accept",
            "application/x-ndjson; profile=\"responses-stream/v2\"",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string("<html>nope</html>"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);
    let result = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("hi")
        .stream(&client.responses())
        .await;
    assert!(result.is_err(), "expected stream content-type error");
    let err = result.err().expect("err");

    match err {
        Error::StreamContentType {
            expected, status, ..
        } => {
            assert_eq!(expected, "application/x-ndjson");
            assert_eq!(status, 200);
        }
        other => panic!("expected StreamContentType error, got {:?}", other),
    }
}

#[tokio::test]
async fn responses_stream_ttft_timeout() {
    let base_url = start_chunked_ndjson_server(
        vec![
            (
                std::time::Duration::from_millis(0),
                json!({"type":"start","request_id":"resp_1","model":"gpt-4o-mini"}).to_string(),
            ),
            (
                std::time::Duration::from_millis(150),
                json!({"type":"completion","content":"Hello"}).to_string(),
            ),
        ],
        None,
    )
    .await;

    let client = Client::new(Config {
        api_key: Some(ApiKey::parse("mr_sk_test_key").unwrap()),
        base_url: Some(base_url),
        retry: Some(RetryConfig {
            max_attempts: 1,
            ..Default::default()
        }),
        ..Default::default()
    })
    .expect("client creation should succeed");

    let mut stream = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("hi")
        .stream_ttft_timeout(std::time::Duration::from_millis(50))
        .stream(&client.responses())
        .await
        .expect("stream should start");

    // First event should arrive (message_start).
    let first = stream.next().await.expect("first item").expect("first ok");
    assert_eq!(first.kind, modelrelay::StreamEventKind::MessageStart);

    // Second read should timeout (TTFT is until first content event).
    let second = stream.next().await.expect("second item");
    match second {
        Err(Error::StreamTimeout(te)) => assert_eq!(te.kind.to_string(), "ttft"),
        Err(other) => panic!("expected stream timeout error, got {:?}", other),
        Ok(evt) => panic!("expected timeout, got event {:?}", evt),
    }
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
        json!({ "type": "update", "patch": [{ "op": "add", "path": "/title", "value": "He" }], "complete_fields": [] }).to_string(),
        json!({ "type": "completion", "payload": { "title": "Hello" }, "complete_fields": ["title"] }).to_string(),
    ]
    .join("\n")
        + "\n";

    Mock::given(method("POST"))
        .and(path("/responses"))
        .and(header(
            "accept",
            "application/x-ndjson; profile=\"responses-stream/v2\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ndjson, "application/x-ndjson"))
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
