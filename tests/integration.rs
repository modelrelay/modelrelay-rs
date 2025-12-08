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

    let customer_id = format!("rust-sdk-customer-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());
    let email = format!("rust-sdk-{}@example.com", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());

    let req = FrontendTokenAutoProvisionRequest::new(
        publishable_key.clone(),
        customer_id.clone(),
        email,
    );

    let token = client
        .auth()
        .frontend_token_auto_provision(req)
        .expect("frontend_token_auto_provision failed");

    assert!(!token.token.is_empty(), "expected non-empty token");
    assert!(token.key_id.is_some(), "expected key_id to be set");
    assert!(token.session_id.is_some(), "expected session_id to be set");
    assert_eq!(token.token_type.as_deref(), Some("Bearer"));

    println!("Rust SDK: Successfully auto-provisioned customer {}", customer_id);
}

#[test]
fn integration_get_token_for_existing_customer() {
    let Some((base_url, publishable_key)) = get_test_config() else {
        eprintln!("Skipping integration test: MODELRELAY_TEST_URL and MODELRELAY_TEST_PUBLISHABLE_KEY not set");
        return;
    };

    use modelrelay::{BlockingClient, BlockingConfig, FrontendTokenAutoProvisionRequest, FrontendTokenRequest};

    let client = BlockingClient::new(BlockingConfig {
        api_key: Some(publishable_key.clone()),
        base_url: Some(base_url),
        ..Default::default()
    })
    .expect("failed to create client");

    let customer_id = format!("rust-sdk-existing-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());
    let email = format!("rust-sdk-existing-{}@example.com", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());

    // First, create the customer with email
    let req = FrontendTokenAutoProvisionRequest::new(
        publishable_key.clone(),
        customer_id.clone(),
        email,
    );
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

    println!("Rust SDK: Successfully got token for existing customer {}", customer_id);
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

    let customer_id = format!("rust-sdk-nonexistent-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());

    let req = FrontendTokenRequest::new(publishable_key.clone(), customer_id);

    let result = client
        .auth()
        .frontend_token(req);

    match result {
        Ok(_) => panic!("Expected EMAIL_REQUIRED error, but got success"),
        Err(modelrelay::Error::Api(api_err)) => {
            assert!(api_err.is_email_required(), "expected EMAIL_REQUIRED error, got: {:?}", api_err);
            assert!(api_err.is_provisioning_error(), "expected provisioning error");
            println!("Rust SDK: Correctly received EMAIL_REQUIRED error");
        }
        Err(other) => panic!("Expected API error, got: {:?}", other),
    }
}
