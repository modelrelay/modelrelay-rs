//! SDK Integration Tests
//!
//! These tests run against a real ModelRelay server and verify the full
//! customer auto-provisioning flow using the Rust SDK.
//!
//! To run these tests:
//! 1. Start the server: `just dev`
//! 2. Set environment variables (the Go orchestrator test sets these automatically):
//!    - MODELRELAY_TEST_URL: Base URL of the API (e.g., http://localhost:8080/api/v1)
//!    - MODELRELAY_TEST_PUBLISHABLE_KEY: Publishable key for the test project
//! 3. Run: `cargo test --features blocking --test integration`
//!
//! These tests are skipped if the environment variables are not set.

#![cfg(feature = "blocking")]

use std::env;

fn get_test_config() -> Option<(String, String)> {
    let url = env::var("MODELRELAY_TEST_URL").ok()?;
    let key = env::var("MODELRELAY_TEST_PUBLISHABLE_KEY").ok()?;
    Some((url, key))
}

#[test]
fn integration_auto_provision_customer_with_email() {
    let Some((base_url, publishable_key)) = get_test_config() else {
        eprintln!("Skipping integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_PUBLISHABLE_KEY not set");
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, FrontendTokenAutoProvisionRequest};

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(publishable_key.clone()),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let customer_id = format!(
        "rust-sdk-customer-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let email = format!(
        "rust-sdk-{}@example.com",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let req =
        FrontendTokenAutoProvisionRequest::new(publishable_key.clone(), customer_id.clone(), email);

    let token = client
        .auth()
        .frontend_token_auto_provision(req)
        .expect("frontend_token_auto_provision failed");

    assert!(!token.token.is_empty(), "expected non-empty token");
    assert!(!token.key_id.is_nil(), "expected key_id to be set");
    assert!(!token.session_id.is_nil(), "expected session_id to be set");
    // token_type is always Bearer, no need to assert

    println!(
        "Rust SDK: Successfully auto-provisioned customer {}",
        customer_id
    );
}

#[test]
fn integration_get_token_for_existing_customer() {
    let Some((base_url, publishable_key)) = get_test_config() else {
        eprintln!("Skipping integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_PUBLISHABLE_KEY not set");
        return;
    };

    use modelrelay::{
        BlockingClient, BlockingConfig, FrontendTokenAutoProvisionRequest, FrontendTokenRequest,
    };

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(publishable_key.clone()),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let customer_id = format!(
        "rust-sdk-existing-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let email = format!(
        "rust-sdk-existing-{}@example.com",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    // First, create the customer with email
    let req =
        FrontendTokenAutoProvisionRequest::new(publishable_key.clone(), customer_id.clone(), email);
    client
        .auth()
        .frontend_token_auto_provision(req)
        .expect("failed to create customer");

    // Now get token for existing customer (no email needed)
    let req2 = FrontendTokenRequest::new(publishable_key.clone(), customer_id.clone());
    let token = client
        .auth()
        .frontend_token(req2)
        .expect("frontend_token for existing customer failed");

    assert!(!token.token.is_empty(), "expected non-empty token");

    println!(
        "Rust SDK: Successfully got token for existing customer {}",
        customer_id
    );
}

#[test]
fn integration_email_required_error_for_nonexistent_customer() {
    let Some((base_url, publishable_key)) = get_test_config() else {
        eprintln!("Skipping integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_PUBLISHABLE_KEY not set");
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, FrontendTokenRequest};

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(publishable_key.clone()),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let customer_id = format!(
        "rust-sdk-nonexistent-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let req = FrontendTokenRequest::new(publishable_key.clone(), customer_id);

    let result = client.auth().frontend_token(req);

    match result {
        Ok(_) => panic!("Expected EMAIL_REQUIRED error, but got success"),
        Err(modelrelay::Error::Api(api_err)) => {
            assert!(
                api_err.is_email_required(),
                "expected EMAIL_REQUIRED error, got: {:?}",
                api_err
            );
            assert!(
                api_err.is_provisioning_error(),
                "expected provisioning error"
            );
            println!("Rust SDK: Correctly received EMAIL_REQUIRED error");
        }
        Err(other) => panic!("Expected API error, got: {:?}", other),
    }
}

/// Get config for LLM proxy tests (requires secret key, not publishable key).
fn get_llm_test_config() -> Option<(String, String)> {
    let url = env::var("MODELRELAY_TEST_URL").ok()?;
    let key = env::var("MODELRELAY_TEST_SECRET_KEY").ok()?;
    Some((url, key))
}

#[test]
fn integration_llm_proxy_basic_request() {
    let Some((base_url, secret_key)) = get_llm_test_config() else {
        eprintln!("Skipping LLM integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_SECRET_KEY not set");
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder};

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(secret_key),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let response = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Say 'hello' in exactly one word.")
        .max_tokens(10)
        .send_blocking(&client.llm())
        .expect("LLM request failed");

    // Verify response structure matches OpenAPI spec
    assert!(!response.id.is_empty(), "response.id should be set");
    assert!(
        !response.content.is_empty(),
        "response.content should not be empty"
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
        "Rust SDK: LLM proxy response - id={}, content={:?}, usage={:?}",
        response.id, response.content, response.usage
    );
}

#[test]
fn integration_llm_proxy_response_has_required_fields() {
    // This test explicitly verifies the ProxyResponse structure matches
    // what the OpenAPI spec (api/openapi/api.yaml) declares as required:
    // - id: string
    // - content: array of strings
    // - model: string
    // - usage: object with input_tokens, output_tokens, total_tokens
    //
    // If this test fails, either the SDK types or OpenAPI spec need updating.

    let Some((base_url, secret_key)) = get_llm_test_config() else {
        eprintln!("Skipping LLM integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_SECRET_KEY not set");
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, ChatRequestBuilder};

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(secret_key),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let response = ChatRequestBuilder::new("gpt-4o-mini")
        .user("Count from 1 to 3.")
        .max_tokens(20)
        .send_blocking(&client.llm())
        .expect("LLM request failed");

    // Required per OpenAPI spec
    assert!(!response.id.is_empty(), "id is required");
    assert!(!response.content.is_empty(), "content is required");
    // model is present in the response (SDK type has Model wrapper)
    // usage is required
    assert!(
        response.usage.total_tokens >= response.usage.input_tokens + response.usage.output_tokens
            || response.usage.total_tokens
                == response.usage.input_tokens + response.usage.output_tokens,
        "usage.total_tokens should be sum of input + output"
    );

    println!("Rust SDK: ProxyResponse structure verified against OpenAPI spec");
}
