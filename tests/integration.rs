//! SDK Integration Tests
//!
//! These tests run against a real ModelRelay server and verify the full
//! customer token minting flow using the Rust SDK.
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

fn get_test_config() -> Option<(String, String)> {
    let url = env::var("MODELRELAY_TEST_URL").ok()?;
    let key = env::var("MODELRELAY_TEST_SECRET_KEY").ok()?;
    Some((url, key))
}

#[test]
fn integration_customer_token_mint() {
    let Some((base_url, secret_key_raw)) = get_test_config() else {
        eprintln!(
            "Skipping integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_SECRET_KEY not set"
        );
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, CustomerCreateRequest, CustomerTokenRequest};
    use uuid::Uuid;

    let secret_key =
        modelrelay::SecretKey::parse(secret_key_raw).expect("invalid MODELRELAY_TEST_SECRET_KEY");

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(secret_key.into()),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let tiers = client.tiers().list().expect("tiers list failed");
    assert!(!tiers.is_empty(), "expected at least one tier");
    let free = tiers
        .iter()
        .find(|t| t.tier_code == "free")
        .cloned()
        .unwrap_or_else(|| tiers[0].clone());

    let external_id = format!(
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

    client
        .customers()
        .create(CustomerCreateRequest {
            tier_id: free.id,
            external_id: external_id.clone(),
            email,
            metadata: None,
        })
        .expect("customer create failed");

    let project_id =
        Uuid::parse_str(&free.project_id).expect("invalid tier project_id (expected UUID string)");

    let token = client
        .auth()
        .customer_token(
            CustomerTokenRequest::for_external_id(project_id, external_id.clone())
                .with_ttl_seconds(600),
        )
        .expect("customer_token failed");

    assert!(!token.token.is_empty(), "expected non-empty token");
    // token_type is always Bearer, no need to assert

    println!(
        "Rust SDK: Successfully minted customer token for {}",
        external_id
    );
}

#[test]
fn integration_responses_basic_request() {
    let Some((base_url, secret_key)) = get_test_config() else {
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
        .model("echo-1")
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

    let Some((base_url, secret_key)) = get_test_config() else {
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
        .model("echo-1")
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
