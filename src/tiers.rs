//! Tier querying API client.
//!
//! Works with both publishable keys (`mr_pk_*`) and secret keys (`mr_sk_*`).

use std::sync::Arc;

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::{
    client::ClientInner,
    errors::{Error, Result, ValidationError},
    http::HeaderList,
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
    pub id: String,
    pub project_id: String,
    pub tier_code: String,
    pub display_name: String,
    pub spend_limit_cents: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_price_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_amount: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_interval: Option<PriceInterval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial_days: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
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
    fn ensure_api_key(&self) -> Result<()> {
        match &self.inner.api_key {
            Some(key) if key.starts_with("mr_pk_") || key.starts_with("mr_sk_") => Ok(()),
            Some(_) => Err(Error::Validation(ValidationError::new(
                "API key (mr_pk_* or mr_sk_*) required for tier operations",
            ))),
            None => Err(Error::Validation(ValidationError::new(
                "api key required for tier operations",
            ))),
        }
    }

    /// List all tiers in the project.
    pub async fn list(&self) -> Result<Vec<Tier>> {
        self.ensure_api_key()?;
        let builder = self.inner.request(Method::GET, "/tiers")?;
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self
            .inner
            .make_context(&Method::GET, "/tiers", None, None, None);
        let resp: TierListResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.tiers)
    }

    /// Get a tier by ID.
    pub async fn get(&self, tier_id: &str) -> Result<Tier> {
        self.ensure_api_key()?;
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
        let ctx = self
            .inner
            .make_context(&Method::GET, &path, None, None, None);
        let resp: TierResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp.tier)
    }
}
