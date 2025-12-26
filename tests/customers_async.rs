use chrono::{DateTime, Utc};
use modelrelay::{
    testing::test_client, Client, Config, CustomerClaimRequest, CustomerCreateRequest,
    CustomerSubscribeRequest, CustomerUpsertRequest, RetryConfig,
};
use serde_json::json;
use uuid::Uuid;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn customers_crud_and_subscription_async() {
    let server = MockServer::start().await;
    let customer_id = Uuid::new_v4();
    let tier_id = Uuid::new_v4();
    let now: DateTime<Utc> = Utc::now();

    Mock::given(method("GET"))
        .and(path("/customers"))
        .and(header("x-modelrelay-api-key", "mr_sk_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "customers": [
                {
                    "customer": {
                        "id": customer_id,
                        "project_id": Uuid::new_v4(),
                        "external_id": "ext-1",
                        "email": "user@example.com",
                        "created_at": now,
                        "updated_at": now
                    }
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/customers"))
        .and(body_json(
            json!({"external_id": "ext-1", "email": "user@example.com"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "customer": {
                "customer": {
                    "id": customer_id,
                    "project_id": Uuid::new_v4(),
                    "external_id": "ext-1",
                    "email": "user@example.com",
                    "created_at": now,
                    "updated_at": now
                }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path("/customers"))
        .and(body_json(
            json!({"external_id": "ext-1", "email": "user@example.com"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "customer": {
                "customer": {
                    "id": customer_id,
                    "project_id": Uuid::new_v4(),
                    "external_id": "ext-1",
                    "email": "user@example.com",
                    "created_at": now,
                    "updated_at": now
                }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/customers/{customer_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "customer": {
                "customer": {
                    "id": customer_id,
                    "project_id": Uuid::new_v4(),
                    "external_id": "ext-1",
                    "email": "user@example.com",
                    "created_at": now,
                    "updated_at": now
                }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path(format!("/customers/{customer_id}")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/customers/claim"))
        .and(body_json(
            json!({"email": "user@example.com", "provider": "oidc", "subject": "sub-1"}),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(format!("/customers/{customer_id}/subscribe")))
        .and(body_json(json!({"tier_id": tier_id, "success_url": "https://example.com/s", "cancel_url": "https://example.com/c"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"session_id": "sess_1", "url": "https://stripe.example/checkout"})))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/customers/{customer_id}/subscription")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subscription": {
                "id": Uuid::new_v4(),
                "project_id": Uuid::new_v4(),
                "customer_id": customer_id,
                "tier_id": tier_id,
                "created_at": now,
                "updated_at": now
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path(format!("/customers/{customer_id}/subscription")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());

    let customers = client.customers().list().await.expect("list customers");
    assert_eq!(customers.len(), 1);

    let created = client
        .customers()
        .create(CustomerCreateRequest {
            external_id: "ext-1".into(),
            email: "user@example.com".into(),
            metadata: None,
        })
        .await
        .expect("create");
    assert_eq!(created.customer.id, customer_id);

    let fetched = client.customers().get(customer_id).await.expect("get");
    assert_eq!(fetched.customer.id, customer_id);

    let upserted = client
        .customers()
        .upsert(CustomerUpsertRequest {
            external_id: "ext-1".into(),
            email: "user@example.com".into(),
            metadata: None,
        })
        .await
        .expect("upsert");
    assert_eq!(upserted.customer.id, customer_id);

    client
        .customers()
        .claim(CustomerClaimRequest {
            email: "user@example.com".into(),
            provider: "oidc".into(),
            subject: "sub-1".into(),
        })
        .await
        .expect("claim");

    client
        .customers()
        .delete(customer_id)
        .await
        .expect("delete");

    let checkout = client
        .customers()
        .subscribe(
            customer_id,
            CustomerSubscribeRequest {
                tier_id,
                success_url: "https://example.com/s".into(),
                cancel_url: "https://example.com/c".into(),
            },
        )
        .await
        .expect("subscribe");
    assert_eq!(checkout.session_id, "sess_1");

    client
        .customers()
        .get_subscription(customer_id)
        .await
        .expect("get subscription");

    client
        .customers()
        .unsubscribe(customer_id)
        .await
        .expect("unsubscribe");
}

#[tokio::test]
async fn customers_me_endpoints_async() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/customers/me"))
        .and(header("authorization", "Bearer header.payload.signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "customer": { "customer": { "id": Uuid::new_v4() } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/customers/me/subscription"))
        .and(header("authorization", "Bearer header.payload.signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subscription": { "tier_code": "pro", "tier_display_name": "Pro" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/customers/me/usage"))
        .and(header("authorization", "Bearer header.payload.signature"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "usage": { "window_start": "2025-01-01T00:00:00Z", "window_end": "2025-02-01T00:00:00Z", "requests": 1, "tokens": 2, "images": 0, "daily": [] }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::new(Config {
        base_url: Some(server.uri()),
        access_token: Some("header.payload.signature".into()),
        retry: Some(RetryConfig::disabled()),
        ..Default::default()
    })
    .expect("client");

    client.customers().me().await.expect("me");
    client.customers().me_subscription().await.expect("me sub");
    client.customers().me_usage().await.expect("me usage");
}
