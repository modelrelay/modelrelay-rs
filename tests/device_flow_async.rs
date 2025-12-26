use modelrelay::{
    testing::test_client, ApiKey, Client, Config, DeviceFlowErrorKind, DeviceFlowProvider,
    DeviceStartRequest, DeviceTokenResult,
};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn device_flow_start_and_token_async() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth/device/start"))
        .and(query_param("provider", "github"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dev-code",
            "user_code": "USER-CODE",
            "verification_uri": "https://example.com/device",
            "verification_uri_complete": "https://example.com/device?code=USER-CODE",
            "expires_in": 600,
            "interval": 5
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/auth/device/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "customer-token",
            "expires_at": "2025-01-01T00:00:00Z",
            "expires_in": 600,
            "project_id": "11111111-1111-1111-1111-111111111111",
            "customer_id": "22222222-2222-2222-2222-222222222222",
            "customer_external_id": "ext_1",
            "tier_code": "pro"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let auth = client
        .auth()
        .device_start(DeviceStartRequest {
            provider: Some(DeviceFlowProvider::Github),
        })
        .await
        .expect("device start");
    assert_eq!(auth.device_code, "dev-code");

    let token = client
        .auth()
        .device_token(&auth.device_code)
        .await
        .expect("device token");

    match token {
        DeviceTokenResult::Approved(token) => {
            assert_eq!(token.token, "customer-token");
        }
        other => panic!("expected approved token, got {other:?}"),
    }
}

#[tokio::test]
async fn device_flow_pending_and_error_async() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth/device/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": "authorization_pending",
            "message": "pending"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let result = client
        .auth()
        .device_token("dev-code")
        .await
        .expect("device token");
    match result {
        DeviceTokenResult::Pending(pending) => {
            assert_eq!(pending.kind, DeviceFlowErrorKind::AuthorizationPending);
        }
        other => panic!("expected pending, got {other:?}"),
    }
}

#[tokio::test]
async fn device_flow_error_async() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth/device/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": "access_denied",
            "message": "denied"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let result = client
        .auth()
        .device_token("dev-code")
        .await
        .expect("device token");
    match result {
        DeviceTokenResult::Error { kind, .. } => {
            assert_eq!(kind, DeviceFlowErrorKind::AccessDenied);
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn device_flow_validates_device_code() {
    let client = Client::new(Config {
        api_key: Some(ApiKey::parse("mr_sk_test").unwrap()),
        ..Default::default()
    })
    .expect("client");

    let err = client
        .auth()
        .device_token(" ")
        .await
        .expect_err("expected validation error");
    match err {
        modelrelay::Error::Validation(v) => {
            assert_eq!(v.field.as_deref(), Some("device_code"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}
