//! Customer management API client.
//!
//! Requires a secret key (`mr_sk_*`) for authentication.

use std::sync::Arc;

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::{
    client::ClientInner,
    errors::{Error, Result, ValidationError},
    http::HeaderList,
};

/// Customer metadata as a key-value map.
pub type CustomerMetadata = std::collections::HashMap<String, serde_json::Value>;

/// A customer in a ModelRelay project.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Customer {
    pub id: String,
    pub project_id: String,
    pub tier_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_code: Option<String>,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_customer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_subscription_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_period_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_period_end: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Request to create a customer.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerCreateRequest {
    pub tier_id: String,
    pub external_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
}

/// Request to upsert a customer by external_id.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerUpsertRequest {
    pub tier_id: String,
    pub external_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<CustomerMetadata>,
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

/// Subscription status response.
#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionStatus {
    pub active: bool,
    #[serde(default)]
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub current_period_start: Option<String>,
    #[serde(default)]
    pub current_period_end: Option<String>,
}

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
    fn ensure_secret_key(&self) -> Result<()> {
        match &self.inner.api_key {
            Some(key) if key.starts_with("mr_sk_") => Ok(()),
            Some(_) => Err(Error::Validation(ValidationError::new(
                "secret key (mr_sk_*) required for customer operations",
            ))),
            None => Err(Error::Validation(ValidationError::new(
                "api key required for customer operations",
            ))),
        }
    }

    /// List all customers in the project.
    pub async fn list(&self) -> Result<Vec<Customer>> {
        self.ensure_secret_key()?;
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
            .make_context(&Method::GET, "/customers", None, None, None);
        let resp: CustomerListResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.customers)
    }

    /// Create a new customer in the project.
    pub async fn create(&self, req: CustomerCreateRequest) -> Result<Customer> {
        self.ensure_secret_key()?;
        if req.tier_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("tier_id is required").with_field("tier_id"),
            ));
        }
        if req.external_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("external_id is required").with_field("external_id"),
            ));
        }
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
            .make_context(&Method::POST, "/customers", None, None, None);
        let resp: CustomerResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Get a customer by ID.
    pub async fn get(&self, customer_id: &str) -> Result<Customer> {
        self.ensure_secret_key()?;
        if customer_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("customer_id is required").with_field("customer_id"),
            ));
        }
        let path = format!("/customers/{}", customer_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, &path, None, None, None);
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
        self.ensure_secret_key()?;
        if req.tier_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("tier_id is required").with_field("tier_id"),
            ));
        }
        if req.external_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("external_id is required").with_field("external_id"),
            ));
        }
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
            .make_context(&Method::PUT, "/customers", None, None, None);
        let resp: CustomerResponse = self
            .inner
            .execute_json(builder, Method::PUT, None, ctx)
            .await?;
        Ok(resp.customer)
    }

    /// Delete a customer by ID.
    pub async fn delete(&self, customer_id: &str) -> Result<()> {
        self.ensure_secret_key()?;
        if customer_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("customer_id is required").with_field("customer_id"),
            ));
        }
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
        let ctx = self
            .inner
            .make_context(&Method::DELETE, &path, None, None, None);
        let _ = self
            .inner
            .send_with_retry(builder, Method::DELETE, retry_cfg, ctx)
            .await?;
        Ok(())
    }

    /// Create a Stripe checkout session for a customer.
    pub async fn create_checkout_session(
        &self,
        customer_id: &str,
        req: CheckoutSessionRequest,
    ) -> Result<CheckoutSession> {
        self.ensure_secret_key()?;
        if customer_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("customer_id is required").with_field("customer_id"),
            ));
        }
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
        let ctx = self
            .inner
            .make_context(&Method::POST, &path, None, None, None);
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }

    /// Get the subscription status for a customer.
    pub async fn get_subscription(&self, customer_id: &str) -> Result<SubscriptionStatus> {
        self.ensure_secret_key()?;
        if customer_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("customer_id is required").with_field("customer_id"),
            ));
        }
        let path = format!("/customers/{}/subscription", customer_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, &path, None, None, None);
        self.inner
            .execute_json(builder, Method::GET, None, ctx)
            .await
    }
}
