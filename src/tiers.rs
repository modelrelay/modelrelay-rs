//! Tier querying API client.
//!
//! Works with both publishable keys (`mr_pk_*`) and secret keys (`mr_sk_*`).

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

/// Billing interval for a tier.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PriceInterval {
    Month,
    Year,
}

/// A pricing tier in a ModelRelay project.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tier {
    pub id: Uuid,
    pub project_id: Uuid,
    pub tier_code: TierCode,
    pub display_name: String,
    pub spend_limit_cents: u64,
    /// Input token price in cents per million (e.g., 300 = $3.00/1M tokens)
    pub input_price_per_million_cents: u64,
    /// Output token price in cents per million (e.g., 1500 = $15.00/1M tokens)
    pub output_price_per_million_cents: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_price_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_amount: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_interval: Option<PriceInterval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial_days: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to create a tier checkout session (Stripe-first flow).
/// Stripe collects the customer's email during checkout.
#[derive(Debug, Clone, Serialize)]
pub struct TierCheckoutRequest {
    pub success_url: String,
    pub cancel_url: String,
}

/// Tier checkout session response.
#[derive(Debug, Clone, Deserialize)]
pub struct TierCheckoutSession {
    pub session_id: String,
    pub url: String,
}

#[derive(Deserialize)]
struct TierListResponse {
    tiers: Vec<Tier>,
}

#[derive(Deserialize)]
struct TierResponse {
    tier: Tier,
}

/// Client for tier querying operations.
///
/// Works with both publishable keys (`mr_pk_*`) and secret keys (`mr_sk_*`).
#[derive(Clone)]
pub struct TiersClient {
    pub(crate) inner: Arc<ClientInner>,
}

impl TiersClient {
    /// List all tiers in the project.
    pub async fn list(&self) -> Result<Vec<Tier>> {
        crate::core::validate_api_key(&self.inner.api_key)?;
        let builder = self.inner.request(Method::GET, "/tiers")?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, "/tiers", None, None);
        let resp: TierListResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.tiers)
    }

    /// Get a tier by ID.
    pub async fn get(&self, tier_id: &str) -> Result<Tier> {
        crate::core::validate_api_key(&self.inner.api_key)?;
        if tier_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("tier_id is required").with_field("tier_id"),
            ));
        }
        let path = format!("/tiers/{}", tier_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp: TierResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.tier)
    }

    /// Create a Stripe checkout session for a tier (Stripe-first flow).
    ///
    /// This enables users to subscribe before authenticating. Stripe collects
    /// the customer's email during checkout. After checkout completes, a
    /// customer record is created with the email from Stripe. The customer
    /// can later be linked to an identity via `CustomersClient::claim`.
    ///
    /// Requires a secret key (`mr_sk_*`).
    pub async fn checkout(
        &self,
        tier_id: &str,
        req: TierCheckoutRequest,
    ) -> Result<TierCheckoutSession> {
        crate::core::validate_secret_key(&self.inner.api_key)?;
        if tier_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("tier_id is required").with_field("tier_id"),
            ));
        }
        if req.success_url.trim().is_empty() || req.cancel_url.trim().is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "success_url and cancel_url are required",
            )));
        }
        let path = format!("/tiers/{}/checkout", tier_id);
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
        let resp: TierCheckoutSession = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }
}
