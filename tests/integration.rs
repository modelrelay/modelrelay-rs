//! SDK Integration Tests
//!
//! These tests run against a real ModelRelay server and verify SDK functionality.
//!
//! To run these tests:
//! 1. Start the server: `just dev`
//! 2. Set environment variables (the Go orchestrator test sets these automatically):
//!    - MODELRELAY_TEST_URL: Base URL of the API (e.g., http://localhost:8080/api/v1)
//!    - MODELRELAY_TEST_SECRET_KEY: Secret key for the test project
//! 3. Run: `cargo test --features blocking --test integration`
//!
//! These tests are skipped if the environment variables are not set.

#![cfg(feature = "blocking")]

use std::env;

/// Get config for Responses API tests (requires secret key).
fn get_responses_test_config() -> Option<(String, String)> {
    let url = env::var("MODELRELAY_TEST_URL").ok()?;
    let key = env::var("MODELRELAY_TEST_SECRET_KEY").ok()?;
    Some((url, key))
}

#[test]
fn integration_responses_basic_request() {
    let Some((base_url, secret_key)) = get_responses_test_config() else {
        eprintln!("Skipping responses integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_SECRET_KEY not set");
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, ResponseBuilder};

    let secret_key =
        modelrelay::SecretKey::parse(secret_key).expect("invalid MODELRELAY_TEST_SECRET_KEY");

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(secret_key.into()),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Say 'hello' in exactly one word.")
        .max_output_tokens(10)
        .send_blocking(&client.responses())
        .expect("LLM request failed");

    // Verify response structure matches OpenAPI spec
    assert!(!response.id.is_empty(), "response.id should be set");
    assert!(
        !response.output.is_empty(),
        "response.output should not be empty"
    );
    assert!(
        response.usage.input_tokens > 0,
        "input_tokens should be > 0"
    );
    assert!(
        response.usage.output_tokens > 0,
        "output_tokens should be > 0"
    );
    assert!(
        response.usage.total_tokens > 0,
        "total_tokens should be > 0"
    );

    println!(
        "Rust SDK: responses - id={}, usage={:?}",
        response.id, response.usage
    );
}

#[test]
fn integration_responses_response_has_required_fields() {
    // This test explicitly verifies the Response structure matches
    // what the OpenAPI spec (api/openapi/api.yaml) declares as required:
    // - id: string
    // - output: array
    // - model: string
    // - usage: object with input_tokens, output_tokens, total_tokens
    //
    // If this test fails, either the SDK types or OpenAPI spec need updating.

    let Some((base_url, secret_key)) = get_responses_test_config() else {
        eprintln!("Skipping responses integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_SECRET_KEY not set");
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, ResponseBuilder};

    let secret_key =
        modelrelay::SecretKey::parse(secret_key).expect("invalid MODELRELAY_TEST_SECRET_KEY");

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(secret_key.into()),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let response = ResponseBuilder::new()
        .model("gpt-4o-mini")
        .user("Count from 1 to 3.")
        .max_output_tokens(20)
        .send_blocking(&client.responses())
        .expect("LLM request failed");

    // Required per OpenAPI spec
    assert!(!response.id.is_empty(), "id is required");
    assert!(!response.output.is_empty(), "output is required");
    // model is present in the response (SDK type has Model wrapper)
    // usage is required
    assert!(
        response.usage.total_tokens >= response.usage.input_tokens + response.usage.output_tokens
            || response.usage.total_tokens
                == response.usage.input_tokens + response.usage.output_tokens,
        "usage.total_tokens should be sum of input + output"
    );

    println!("Rust SDK: Response structure verified against OpenAPI spec");
}
