//! Customer management API client.
//!
//! Requires a secret key (`mr_sk_*`) for authentication.

use std::{collections::HashMap, sync::Arc};

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use uuid::Uuid;

use crate::{
    client::ClientInner,
    errors::{Error, Result, ValidationError},
    http::HeaderList,
    identifiers::TierCode,
};

pub type CustomerMe = crate::generated::CustomerMe;
pub type CustomerMeUsage = crate::generated::CustomerMeUsage;
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

/// Subscription status values.
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

/// Billing providers that can back a subscription.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingProvider {
    Stripe,
    Crypto,
    AppStore,
    External,
    /// Unknown provider for forward compatibility with new values.
    #[serde(other)]
    Unknown,
}

/// Customer metadata value without untyped JSON values.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum CustomerMetadataValue {
    String(String),
    Number(Number),
    Bool(bool),
    Null,
    Array(Vec<CustomerMetadataValue>),
    Object(HashMap<String, CustomerMetadataValue>),
}

/// Customer metadata as a typed key-value map.
pub type CustomerMetadata = HashMap<String, CustomerMetadataValue>;

/// A customer in a ModelRelay project.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Customer {
    pub id: Uuid,
    pub project_id: Uuid,
    pub external_id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Subscription details for a customer.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Subscription {
    pub id: Uuid,
    pub project_id: Uuid,
    pub customer_id: Uuid,
    pub tier_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_code: Option<TierCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_provider: Option<BillingProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_customer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_subscription_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_status: Option<SubscriptionStatusKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_period_start: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_period_end: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// CustomerWithSubscription bundles customer identity with optional subscription state.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomerWithSubscription {
    pub customer: Customer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Subscription>,
}

/// Request to create a customer.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerCreateRequest {
    pub external_id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
}

/// Request to upsert a customer by external_id.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerUpsertRequest {
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

/// Request to link a customer identity (provider + subject) to a customer by email.
///
/// Used when a customer subscribes via Stripe Checkout (email only) and later authenticates to the app.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerClaimRequest {
    pub email: String,
    pub provider: String,
    pub subject: String,
}

/// Request to create a customer subscription checkout session.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerSubscribeRequest {
    pub tier_id: Uuid,
    pub success_url: String,
    pub cancel_url: String,
}

/// Checkout session response.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckoutSession {
    pub session_id: String,
    pub url: String,
}

#[derive(Deserialize)]
struct CustomerListResponse {
    customers: Vec<CustomerWithSubscription>,
}

#[derive(Deserialize)]
struct CustomerResponse {
    customer: CustomerWithSubscription,
}

#[derive(Deserialize)]
struct CustomerSubscriptionResponse {
    subscription: Subscription,
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

    /// Get the authenticated customer's usage metrics for the current billing window.
    ///
    /// This endpoint requires a customer bearer token. API keys are not accepted.
    pub async fn me_usage(&self) -> Result<CustomerMeUsage> {
        if !self.inner.has_jwt_access_token() {
            return Err(Error::Validation(ValidationError::new(
                "access token (customer bearer token) is required",
            )));
        }

        let builder = self.inner.request(Method::GET, "/customers/me/usage")?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, "/customers/me/usage", None, None);
        let resp: crate::generated::CustomerMeUsageResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.usage)
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
    pub async fn list(&self) -> Result<Vec<CustomerWithSubscription>> {
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
    pub async fn create(&self, req: CustomerCreateRequest) -> Result<CustomerWithSubscription> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        validate_customer_request(&req)?;
        let mut builder = self.inner.request(Method::POST, "/customers")?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
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
    pub async fn get(&self, customer_id: Uuid) -> Result<CustomerWithSubscription> {
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
    /// If a customer with the given external_id exists, it is updated.
    /// Otherwise, a new customer is created.
    pub async fn upsert(&self, req: CustomerUpsertRequest) -> Result<CustomerWithSubscription> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        validate_customer_request(&req)?;
        let mut builder = self.inner.request(Method::PUT, "/customers")?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::PUT, "/customers", None, None);
        let resp: CustomerResponse = self
            .inner
            .execute_json(builder, Method::PUT, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Link a customer identity (provider + subject) to a customer found by email.
    ///
    /// Used when a customer subscribes via Stripe Checkout (email only) and later authenticates
    /// to the app.
    ///
    /// This is a user self-service operation that works with publishable keys,
    /// allowing CLI tools and frontends to link subscriptions to user identities.
    ///
    /// Errors:
    /// - Customer not found by email (404)
    /// - Identity already linked to a different customer (409)
    pub async fn claim(&self, req: CustomerClaimRequest) -> Result<()> {
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
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let retry_cfg = self.inner.retry.clone();
        let ctx = self
            .inner
            .make_context(&Method::POST, "/customers/claim", None, None);
        let _ = self
            .inner
            .send_with_retry(builder, Method::POST, retry_cfg, ctx)
            .await?;
        Ok(())
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

    /// Create a Stripe checkout session for a customer subscription.
    pub async fn subscribe(
        &self,
        customer_id: Uuid,
        req: CustomerSubscribeRequest,
    ) -> Result<CheckoutSession> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        if req.tier_id == Uuid::nil() {
            return Err(Error::Validation(ValidationError::new(
                "tier_id is required",
            )));
        }
        if req.success_url.trim().is_empty() || req.cancel_url.trim().is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "success_url and cancel_url are required",
            )));
        }
        let path = format!("/customers/{}/subscribe", customer_id);
        let mut builder = self.inner.request(Method::POST, &path)?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, &path, None, None);
        let resp: CheckoutSession = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }

    /// Get the subscription details for a customer.
    pub async fn get_subscription(&self, customer_id: Uuid) -> Result<Subscription> {
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
        let resp: CustomerSubscriptionResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.subscription)
    }

    /// Cancel a customer's subscription at period end.
    pub async fn unsubscribe(&self, customer_id: Uuid) -> Result<()> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        let path = format!("/customers/{}/subscription", customer_id);
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
}
