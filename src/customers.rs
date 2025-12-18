//! Customer management API client.
//!
//! Requires a secret key (`mr_sk_*`) for authentication.

use std::sync::Arc;

use reqwest::Method;
use serde::{Deserialize, Serialize};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    client::ClientInner,
    errors::{Error, Result, ValidationError},
    http::HeaderList,
    identifiers::TierCode,
};

pub type CustomerMe = crate::generated::CustomerMe;
pub type CustomerMeSubscription = crate::generated::CustomerMeSubscription;

/// Simple email validation - checks for basic email format (contains @ with text on both sides and a dot in domain).
pub(crate) fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// Trait for customer request types that share common validation.
pub(crate) trait CustomerRequestFields {
    fn external_id(&self) -> &str;
    fn email(&self) -> &str;
}

/// Validates common customer request fields (external_id, email).
/// Note: tier_id is a Uuid type and is always valid when present.
pub(crate) fn validate_customer_request<T: CustomerRequestFields>(req: &T) -> Result<()> {
    if req.external_id().trim().is_empty() {
        return Err(Error::Validation(
            ValidationError::new("external_id is required").with_field("external_id"),
        ));
    }
    if req.email().trim().is_empty() {
        return Err(Error::Validation(
            ValidationError::new("email is required").with_field("email"),
        ));
    }
    if !is_valid_email(req.email()) {
        return Err(Error::Validation(
            ValidationError::new("invalid email format").with_field("email"),
        ));
    }
    Ok(())
}

/// Subscription status values (matches Stripe subscription statuses).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatusKind {
    Active,
    PastDue,
    Canceled,
    Trialing,
    Incomplete,
    IncompleteExpired,
    Unpaid,
    Paused,
    /// Unknown status for forward compatibility with new Stripe statuses.
    #[serde(other)]
    Unknown,
}

/// Customer metadata as a key-value map.
pub type CustomerMetadata = std::collections::HashMap<String, serde_json::Value>;

/// A customer in a ModelRelay project.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Customer {
    pub id: Uuid,
    pub project_id: Uuid,
    pub tier_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_code: Option<TierCode>,
    pub external_id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_customer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_subscription_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_status: Option<SubscriptionStatusKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_period_start: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_period_end: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to create a customer.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerCreateRequest {
    pub tier_id: Uuid,
    pub external_id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
}

/// Request to upsert a customer by external_id.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerUpsertRequest {
    pub tier_id: Uuid,
    pub external_id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
}

impl CustomerRequestFields for CustomerCreateRequest {
    fn external_id(&self) -> &str {
        &self.external_id
    }
    fn email(&self) -> &str {
        &self.email
    }
}

impl CustomerRequestFields for CustomerUpsertRequest {
    fn external_id(&self) -> &str {
        &self.external_id
    }
    fn email(&self) -> &str {
        &self.email
    }
}

/// Request to link an end-user identity (provider + subject) to a customer by email.
///
/// Used when a customer subscribes via Stripe Checkout (email only) and later authenticates to the app.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerClaimRequest {
    pub email: String,
    pub provider: String,
    pub subject: String,
}

/// Request to create a checkout session.
#[derive(Debug, Clone, Serialize)]
pub struct CheckoutSessionRequest {
    pub success_url: String,
    pub cancel_url: String,
}

/// Checkout session response.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckoutSession {
    pub session_id: String,
    pub url: String,
}

// SubscriptionStatus is now generated from OpenAPI spec with x-rust-type extension
// for the `status` field, which references the hand-written SubscriptionStatusKind enum.
pub use crate::generated::SubscriptionStatus;

#[derive(Deserialize)]
struct CustomerListResponse {
    customers: Vec<Customer>,
}

#[derive(Deserialize)]
struct CustomerResponse {
    customer: Customer,
}

/// Client for customer management operations.
///
/// Requires a secret key (`mr_sk_*`) for authentication.
#[derive(Clone)]
pub struct CustomersClient {
    pub(crate) inner: Arc<ClientInner>,
}

impl CustomersClient {
    /// Get the authenticated customer from a customer-scoped bearer token.
    ///
    /// This endpoint requires a customer bearer token. API keys are not accepted.
    pub async fn me(&self) -> Result<CustomerMe> {
        if !self.inner.has_jwt_access_token() {
            return Err(Error::Validation(ValidationError::new(
                "access token (customer bearer token) is required",
            )));
        }

        let builder = self.inner.request(Method::GET, "/customers/me")?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, "/customers/me", None, None);
        let resp: crate::generated::CustomerMeResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Get the authenticated customer's subscription details.
    ///
    /// This endpoint requires a customer bearer token. API keys are not accepted.
    pub async fn me_subscription(&self) -> Result<CustomerMeSubscription> {
        if !self.inner.has_jwt_access_token() {
            return Err(Error::Validation(ValidationError::new(
                "access token (customer bearer token) is required",
            )));
        }

        let builder = self
            .inner
            .request(Method::GET, "/customers/me/subscription")?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, "/customers/me/subscription", None, None);
        let resp: crate::generated::CustomerMeSubscriptionResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.subscription)
    }

    /// List all customers in the project.
    pub async fn list(&self) -> Result<Vec<Customer>> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        let builder = self.inner.request(Method::GET, "/customers")?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, "/customers", None, None);
        let resp: CustomerListResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.customers)
    }

    /// Create a new customer in the project.
    pub async fn create(&self, req: CustomerCreateRequest) -> Result<Customer> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        validate_customer_request(&req)?;
        let mut builder = self.inner.request(Method::POST, "/customers")?;
        builder = builder.json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::POST, "/customers", None, None);
        let resp: CustomerResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Get a customer by ID.
    pub async fn get(&self, customer_id: Uuid) -> Result<Customer> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        let path = format!("/customers/{}", customer_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp: CustomerResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Upsert a customer by external_id.
    ///
    /// If a customer with the given external_id exists, it is updated.
    /// Otherwise, a new customer is created.
    pub async fn upsert(&self, req: CustomerUpsertRequest) -> Result<Customer> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        validate_customer_request(&req)?;
        let mut builder = self.inner.request(Method::PUT, "/customers")?;
        builder = builder.json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::PUT, "/customers", None, None);
        let resp: CustomerResponse = self
            .inner
            .execute_json(builder, Method::PUT, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Link an end-user identity (provider + subject) to a customer found by email.
    ///
    /// Used when a customer subscribes via Stripe Checkout (email only) and later authenticates
    /// to the app. This is a user self-service operation that works with publishable keys,
    /// allowing CLI tools and frontends to link subscriptions to user identities.
    ///
    /// Works with both publishable keys (`mr_pk_*`) and secret keys (`mr_sk_*`).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Customer not found by email (404)
    /// - Identity already linked to a different customer (409)
    pub async fn claim(&self, req: CustomerClaimRequest) -> Result<Customer> {
        crate::core::validate_api_key(&self.inner.api_key)?;
        if req.email.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("email is required").with_field("email"),
            ));
        }
        if !is_valid_email(&req.email) {
            return Err(Error::Validation(
                ValidationError::new("invalid email format").with_field("email"),
            ));
        }
        if req.provider.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("provider is required").with_field("provider"),
            ));
        }
        if req.subject.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("subject is required").with_field("subject"),
            ));
        }
        let mut builder = self.inner.request(Method::POST, "/customers/claim")?;
        builder = builder.json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::POST, "/customers/claim", None, None);
        let resp: CustomerResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Delete a customer by ID.
    pub async fn delete(&self, customer_id: Uuid) -> Result<()> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        let path = format!("/customers/{}", customer_id);
        let builder = self.inner.request(Method::DELETE, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let retry_cfg = self.inner.retry.clone();
        let ctx = self.inner.make_context(&Method::DELETE, &path, None, None);
        let _ = self
            .inner
            .send_with_retry(builder, Method::DELETE, retry_cfg, ctx)
            .await?;
        Ok(())
    }

    /// Create a Stripe checkout session for a customer.
    pub async fn create_checkout_session(
        &self,
        customer_id: Uuid,
        req: CheckoutSessionRequest,
    ) -> Result<CheckoutSession> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        if req.success_url.trim().is_empty() || req.cancel_url.trim().is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "success_url and cancel_url are required",
            )));
        }
        let path = format!("/customers/{}/checkout", customer_id);
        let mut builder = self.inner.request(Method::POST, &path)?;
        builder = builder.json(&req);
        builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, &path, None, None);
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }

    /// Get the subscription status for a customer.
    pub async fn get_subscription(&self, customer_id: Uuid) -> Result<SubscriptionStatus> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        let path = format!("/customers/{}/subscription", customer_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        self.inner
            .execute_json(builder, Method::GET, None, ctx)
            .await
    }
}
