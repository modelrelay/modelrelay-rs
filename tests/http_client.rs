//! HTTP client tests using wiremock mock server.
//!
//! These tests verify the actual HTTP client behavior including:
//! - Request/response serialization
//! - Error handling for various HTTP status codes
//! - Retry logic
//! - Streaming responses

use futures_util::StreamExt;
use modelrelay::{ChatRequestBuilder, Client, Config, Error, RetryConfig};
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn proxy_sends_correct_request_and_parses_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("X-ModelRelay-Api-Key", "mr_sk_test_key"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "user", "content": "Hello!"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_123",
            "content": ["Hello! How can I help you?"],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 8,
                "total_tokens": 18
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello!")
        .send(&client.llm())
        .await
        .expect("request should succeed");

    assert_eq!(response.id, "resp_123");
    assert_eq!(response.content.join(""), "Hello! How can I help you?");
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 8);
}

#[tokio::test]
async fn proxy_includes_optional_parameters() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(body_json(json!({
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hi"}
            ],
            "max_tokens": 100,
            "temperature": 0.7,
            "stop": ["END"]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_456",
            "content": ["Hi there!"],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": {"input_tokens": 5, "output_tokens": 3, "total_tokens": 8}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response = ChatRequestBuilder::new("gpt-4o-mini")
        .system("You are helpful.")
        .user("Hi")
        .max_tokens(100)
        .temperature(0.7)
        .stop(vec!["END".into()])
        .send(&client.llm())
        .await
        .expect("request should succeed");

    assert_eq!(response.id, "resp_456");
}

#[tokio::test]
async fn proxy_handles_api_error_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "code": "INVALID_REQUEST",
                "message": "Invalid model specified"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let result = ChatRequestBuilder::new("invalid-model")
        .user("Hello")
        .send(&client.llm())
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
async fn proxy_handles_rate_limit_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {
                "code": "RATE_LIMITED",
                "message": "Too many requests"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let result = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello")
        .send(&client.llm())
        .await;

    match result {
        Err(Error::Api(api_err)) => {
            assert_eq!(api_err.status, 429);
            assert_eq!(api_err.code.as_deref(), Some("RATE_LIMITED"));
        }
        other => panic!("expected API error, got {:?}", other),
    }
}

#[tokio::test]
async fn proxy_handles_server_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {
                "code": "INTERNAL_ERROR",
                "message": "Something went wrong"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let result = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello")
        .send(&client.llm())
        .await;

    match result {
        Err(Error::Api(api_err)) => {
            assert_eq!(api_err.status, 500);
        }
        other => panic!("expected API error, got {:?}", other),
    }
}

#[tokio::test]
async fn proxy_retries_on_server_error() {
    let server = MockServer::start().await;

    // First request fails with 500
    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"message": "Server error"}
        })))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    // Second request succeeds
    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_retry",
            "content": ["Success after retry"],
            "model": "gpt-4o-mini",
            "stop_reason": "stop",
            "usage": {"input_tokens": 5, "output_tokens": 3, "total_tokens": 8}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new(Config {
        api_key: Some("mr_sk_test_key".into()),
        base_url: Some(server.uri()),
        retry: Some(RetryConfig {
            max_attempts: 2,
            base_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
            retry_post: true,
        }),
        ..Default::default()
    })
    .expect("client creation should succeed");

    let response = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello")
        .send(&client.llm())
        .await
        .expect("request should succeed after retry");

    assert_eq!(response.id, "resp_retry");
}

#[tokio::test]
async fn proxy_includes_request_id_header() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("X-ModelRelay-Chat-Request-Id", "custom-req-123"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-ModelRelay-Chat-Request-Id", "custom-req-123")
                .set_body_json(json!({
                    "id": "resp_with_req_id",
                    "content": ["Response"],
                    "model": "gpt-4o-mini",
                    "stop_reason": "stop",
                    "usage": {"input_tokens": 5, "output_tokens": 1, "total_tokens": 6}
                })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello")
        .request_id("custom-req-123")
        .send(&client.llm())
        .await
        .expect("request should succeed");

    assert_eq!(response.request_id.as_deref(), Some("custom-req-123"));
}

#[tokio::test]
async fn proxy_stream_ndjson_yields_events() {
    let server = MockServer::start().await;

    // Unified NDJSON format with "type" field and payload.content for text
    let ndjson_body = r#"{"type":"start","model":"gpt-4o-mini","request_id":"resp_stream"}
{"type":"update","payload":{"content":"Hello"}}
{"type":"update","payload":{"content":"Hello world"}}
{"type":"completion","payload":{"content":"Hello world"},"stop_reason":"stop","usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}
"#;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(ndjson_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let mut stream = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello")
        .stream(&client.llm())
        .await
        .expect("stream should start");

    // In unified format, update events have accumulated content, so we use the last value
    let mut final_text = String::new();
    while let Some(event) = stream.next().await {
        let event = event.expect("event should parse");
        if let Some(ref text) = event.text_delta {
            final_text = text.clone();
        }
    }

    assert_eq!(final_text, "Hello world");
}

#[tokio::test]
async fn proxy_handles_empty_response_body() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .respond_with(ResponseTemplate::new(502))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let result = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello")
        .send(&client.llm())
        .await;

    match result {
        Err(Error::Api(api_err)) => {
            assert_eq!(api_err.status, 502);
        }
        other => panic!("expected API error, got {:?}", other),
    }
}

#[tokio::test]
async fn proxy_handles_malformed_json_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string("not valid json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let result = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Hello")
        .send(&client.llm())
        .await;

    // Should be a serialization error
    assert!(
        matches!(result, Err(Error::Serialization(_))),
        "expected serialization error, got {:?}",
        result
    );
}

#[tokio::test]
async fn auth_frontend_token_request() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth/frontend-token"))
        .and(body_json(json!({
            "publishable_key": "mr_pk_test",
            "customer_id": "customer-123"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "mr_ft_abc123",
            "expires_at": "2025-12-31T23:59:59Z",
            "expires_in": 3600,
            "token_type": "Bearer",
            "key_id": "00000000-0000-0000-0000-000000000001",
            "session_id": "00000000-0000-0000-0000-000000000002",
            "project_id": "00000000-0000-0000-0000-000000000003",
            "customer_id": "00000000-0000-0000-0000-000000000004",
            "customer_external_id": "customer-123",
            "tier_code": "free"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new(Config {
        api_key: Some("mr_pk_test".into()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .expect("client creation should succeed");

    let token = client
        .auth()
        .frontend_token(modelrelay::FrontendTokenRequest::new(
            "mr_pk_test",
            "customer-123",
        ))
        .await
        .expect("frontend token request should succeed");

    assert_eq!(token.token, "mr_ft_abc123");
    assert_eq!(token.customer_external_id, "customer-123");
    assert_eq!(token.tier_code, "free");
}

// ============================================================================
// Structured Streaming NDJSON Tests
// ============================================================================
//
// These tests verify the structured streaming contract documented in
// docs/llm-proxy-streaming.md and implemented in providers/sse/structured_ndjson.go.
//
// The structured NDJSON format uses records with:
// - "type": "start" | "update" | "completion" | "error"
// - "payload": the progressively-built JSON object
// - "complete_fields": array of field paths that are complete
// - "code", "message", "status": for error records
// - "request_id", "provider", "model": metadata

/// Test type for structured streaming tests.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, schemars::JsonSchema)]
struct Article {
    title: String,
    summary: String,
    #[serde(default)]
    body: String,
}

#[tokio::test]
async fn structured_stream_yields_update_and_completion_events() {
    let server = MockServer::start().await;

    // Structured NDJSON format (providers/sse/structured_ndjson.go):
    // - start record initiates the stream
    // - update records contain partial payload with complete_fields
    // - completion record contains final payload
    let ndjson_body = r#"{"type":"start","request_id":"req-struct-1","provider":"openai","model":"gpt-4o-mini"}
{"type":"update","payload":{"title":"Rust Ownership","summary":"","body":""},"complete_fields":["title"]}
{"type":"update","payload":{"title":"Rust Ownership","summary":"A guide to ownership","body":""},"complete_fields":["title","summary"]}
{"type":"completion","payload":{"title":"Rust Ownership","summary":"A guide to ownership","body":"The body content..."}}
"#;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .insert_header("X-ModelRelay-Chat-Request-Id", "req-struct-1")
                .set_body_string(ndjson_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response_format = modelrelay::ResponseFormat::json_schema(
        "Article",
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "summary": {"type": "string"},
                "body": {"type": "string"}
            },
            "required": ["title", "summary", "body"]
        }),
    );

    let mut stream = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Write an article about Rust ownership")
        .response_format(response_format)
        .stream_json::<Article>(&client.llm())
        .await
        .expect("stream should start");

    let mut events = Vec::new();
    while let Ok(Some(event)) = stream.next().await {
        events.push(event);
    }

    // Should have 2 update events and 1 completion event (start is skipped)
    assert_eq!(
        events.len(),
        3,
        "expected 3 events (2 updates + 1 completion)"
    );

    // First update: title complete
    assert_eq!(events[0].kind, modelrelay::StructuredRecordKind::Update);
    assert_eq!(events[0].payload.title, "Rust Ownership");
    assert!(events[0].complete_fields.contains("title"));
    assert!(!events[0].complete_fields.contains("summary"));

    // Second update: title and summary complete
    assert_eq!(events[1].kind, modelrelay::StructuredRecordKind::Update);
    assert!(events[1].complete_fields.contains("title"));
    assert!(events[1].complete_fields.contains("summary"));
    assert!(!events[1].complete_fields.contains("body"));

    // Completion: all fields complete
    assert_eq!(events[2].kind, modelrelay::StructuredRecordKind::Completion);
    assert_eq!(events[2].payload.title, "Rust Ownership");
    assert_eq!(events[2].payload.summary, "A guide to ownership");
    assert_eq!(events[2].payload.body, "The body content...");
}

#[tokio::test]
async fn structured_stream_collect_returns_final_payload() {
    let server = MockServer::start().await;

    let ndjson_body = r#"{"type":"start","request_id":"req-collect"}
{"type":"update","payload":{"title":"Partial","summary":"","body":""},"complete_fields":["title"]}
{"type":"completion","payload":{"title":"Final Title","summary":"Final Summary","body":"Final Body"}}
"#;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(ndjson_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response_format = modelrelay::ResponseFormat::json_schema(
        "Article",
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "summary": {"type": "string"},
                "body": {"type": "string"}
            }
        }),
    );

    let stream = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Write an article")
        .response_format(response_format)
        .stream_json::<Article>(&client.llm())
        .await
        .expect("stream should start");

    // collect() should return the completion payload, not the last update
    let result = stream.collect().await.expect("collect should succeed");

    assert_eq!(result.title, "Final Title");
    assert_eq!(result.summary, "Final Summary");
    assert_eq!(result.body, "Final Body");
}

#[tokio::test]
async fn structured_stream_error_record_returns_api_error() {
    let server = MockServer::start().await;

    // Error record format from providers/sse/structured_ndjson.go
    let ndjson_body = r#"{"type":"start","request_id":"req-error"}
{"type":"error","code":"UPSTREAM_ERROR","message":"Provider returned invalid JSON","status":502}
"#;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .insert_header("X-ModelRelay-Chat-Request-Id", "req-error")
                .set_body_string(ndjson_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response_format =
        modelrelay::ResponseFormat::json_schema("Article", serde_json::json!({"type": "object"}));

    let mut stream = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Generate")
        .response_format(response_format)
        .stream_json::<Article>(&client.llm())
        .await
        .expect("stream should start");

    let result = stream.next().await;

    match result {
        Err(Error::Api(api_err)) => {
            assert_eq!(api_err.status, 502);
            assert_eq!(api_err.code.as_deref(), Some("UPSTREAM_ERROR"));
            assert!(api_err.message.contains("invalid JSON"));
            assert_eq!(api_err.request_id.as_deref(), Some("req-error"));
        }
        Ok(Some(_)) => panic!("expected API error, got Ok(Some(...))"),
        Ok(None) => panic!("expected API error, got Ok(None)"),
        Err(other) => panic!("expected API error, got {:?}", other),
    }
}

#[tokio::test]
async fn structured_stream_without_completion_returns_error() {
    let server = MockServer::start().await;

    // Stream that ends without a completion or error record
    let ndjson_body = r#"{"type":"start","request_id":"req-incomplete"}
{"type":"update","payload":{"title":"Partial","summary":"","body":""},"complete_fields":["title"]}
"#;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(ndjson_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response_format =
        modelrelay::ResponseFormat::json_schema("Article", serde_json::json!({"type": "object"}));

    let stream = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Generate")
        .response_format(response_format)
        .stream_json::<Article>(&client.llm())
        .await
        .expect("stream should start");

    // collect() should fail because stream ended without completion
    let result = stream.collect().await;

    match result {
        Err(Error::Transport(te)) => {
            assert!(
                te.message.contains("without completion"),
                "unexpected error message: {}",
                te.message
            );
        }
        other => panic!("expected Transport error, got {:?}", other),
    }
}

#[tokio::test]
async fn structured_stream_with_nested_payload() {
    let server = MockServer::start().await;

    #[derive(Debug, Clone, serde::Deserialize, PartialEq)]
    struct Author {
        name: String,
        email: String,
    }

    #[derive(Debug, Clone, serde::Deserialize, PartialEq)]
    struct BlogPost {
        title: String,
        author: Author,
        tags: Vec<String>,
    }

    // Nested payload with complete_fields using dot notation
    let ndjson_body = r#"{"type":"start"}
{"type":"update","payload":{"title":"Hello World","author":{"name":"","email":""},"tags":[]},"complete_fields":["title"]}
{"type":"update","payload":{"title":"Hello World","author":{"name":"Jane","email":""},"tags":[]},"complete_fields":["title","author.name"]}
{"type":"completion","payload":{"title":"Hello World","author":{"name":"Jane","email":"jane@example.com"},"tags":["rust","tutorial"]}}
"#;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("accept", "application/x-ndjson"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string(ndjson_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response_format = modelrelay::ResponseFormat::json_schema(
        "BlogPost",
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "author": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "email": {"type": "string"}
                    }
                },
                "tags": {"type": "array", "items": {"type": "string"}}
            }
        }),
    );

    let mut stream = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Generate a blog post")
        .response_format(response_format)
        .stream_json::<BlogPost>(&client.llm())
        .await
        .expect("stream should start");

    let mut events = Vec::new();
    while let Ok(Some(event)) = stream.next().await {
        events.push(event);
    }

    assert_eq!(events.len(), 3);

    // Check nested field completion tracking
    assert!(events[1].complete_fields.contains("author.name"));

    // Final payload should have all nested data
    let final_payload = &events[2].payload;
    assert_eq!(final_payload.title, "Hello World");
    assert_eq!(final_payload.author.name, "Jane");
    assert_eq!(final_payload.author.email, "jane@example.com");
    assert_eq!(final_payload.tags, vec!["rust", "tutorial"]);
}

#[tokio::test]
async fn structured_stream_request_id_propagates() {
    let server = MockServer::start().await;

    let ndjson_body = r#"{"type":"start","request_id":"custom-req-456"}
{"type":"completion","payload":{"title":"Test","summary":"Test","body":"Test"}}
"#;

    Mock::given(method("POST"))
        .and(path("/llm/proxy"))
        .and(header("accept", "application/x-ndjson"))
        .and(header("X-ModelRelay-Chat-Request-Id", "custom-req-456"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .insert_header("X-ModelRelay-Chat-Request-Id", "custom-req-456")
                .set_body_string(ndjson_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let response_format =
        modelrelay::ResponseFormat::json_schema("Article", serde_json::json!({"type": "object"}));

    let mut stream = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Generate")
        .request_id("custom-req-456")
        .response_format(response_format)
        .stream_json::<Article>(&client.llm())
        .await
        .expect("stream should start");

    // Request ID should be available on the stream
    assert_eq!(stream.request_id(), Some("custom-req-456"));

    // Request ID should also be on events
    let event = stream.next().await.unwrap().unwrap();
    assert_eq!(event.request_id.as_deref(), Some("custom-req-456"));
}

#[tokio::test]
async fn structured_stream_requires_json_schema_response_format() {
    let server = MockServer::start().await;

    // No mock needed - validation happens before request

    let client = client_for_server(&server);

    // Without response_format
    let result = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Generate")
        .stream_json::<Article>(&client.llm())
        .await;

    match result {
        Err(Error::Validation(v)) => {
            assert!(
                v.to_string().contains("response_format"),
                "expected response_format error, got: {}",
                v
            );
        }
        Ok(_) => panic!("expected Validation error, got Ok"),
        Err(other) => panic!("expected Validation error, got {:?}", other),
    }

    // With text response_format (not json_schema)
    let result = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Generate")
        .response_format(modelrelay::ResponseFormat::text())
        .stream_json::<Article>(&client.llm())
        .await;

    match result {
        Err(Error::Validation(v)) => {
            assert!(
                v.to_string().contains("structured"),
                "expected structured error, got: {}",
                v
            );
        }
        Ok(_) => panic!("expected Validation error, got Ok"),
        Err(other) => panic!("expected Validation error, got {:?}", other),
    }
}
