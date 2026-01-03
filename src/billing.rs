//! Billing API client for customer self-service operations.
//!
//! This module provides helpers for customer billing operations like viewing
//! subscription status, usage metrics, balance, and managing subscriptions.
//!
//! Requires a customer bearer token for authentication (not API keys).
//!
//! # Example
//!
//! ```rust,ignore
//! use modelrelay::Client;
//!
//! // Customer token from device flow or OIDC exchange
//! let client = Client::from_bearer_token(customer_token)?
//!     .build()?;
//!
//! // Get customer info
//! let me = client.billing().me().await?;
//! println!("Customer: {:?}", me.customer.email);
//! println!("Tier: {:?}", me.tier.as_ref().map(|t| &t.code));
//!
//! // Get subscription details
//! let sub = client.billing().subscription().await?;
//! println!("Tier: {:?}", sub.tier_code);
//!
//! // Get usage metrics
//! let usage = client.billing().usage().await?;
//! println!("Tokens used: {}", usage.total_tokens);
//! ```

use std::sync::Arc;

use reqwest::Method;

use crate::{
    client::ClientInner,
    errors::Result,
    generated::{
        ChangeTierRequest, CheckoutSessionResponse, CustomerBalanceResponse,
        CustomerLedgerResponse, CustomerMe, CustomerMeCheckoutRequest, CustomerMeResponse,
        CustomerMeSubscription, CustomerMeSubscriptionResponse, CustomerMeUsage,
        CustomerMeUsageResponse, CustomerTopupRequest, CustomerTopupResponse,
    },
    http::HeaderList,
};

/// Client for customer billing self-service operations.
///
/// These endpoints require a customer bearer token (from device flow or OIDC exchange).
/// API keys are not accepted.
#[derive(Clone)]
pub struct BillingClient {
    pub(crate) inner: Arc<ClientInner>,
}

impl BillingClient {
    /// Get the authenticated customer's profile.
    ///
    /// Returns customer details including ID, email, external ID, and metadata.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let me = client.billing().me().await?;
    /// println!("Customer ID: {:?}", me.customer.id);
    /// println!("Tier: {:?}", me.tier.as_ref().map(|t| &t.code));
    /// ```
    pub async fn me(&self) -> Result<CustomerMe> {
        let path = "/customers/me";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        let response: CustomerMeResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(response.customer)
    }

    /// Get the authenticated customer's subscription details.
    ///
    /// Returns subscription status, tier information, and billing provider.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let sub = client.billing().subscription().await?;
    /// println!("Tier: {:?}", sub.tier_code);
    /// println!("Status: {:?}", sub.subscription_status);
    /// ```
    pub async fn subscription(&self) -> Result<CustomerMeSubscription> {
        let path = "/customers/me/subscription";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        let response: CustomerMeSubscriptionResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(response.subscription)
    }

    /// Get the authenticated customer's usage metrics.
    ///
    /// Returns token usage, request counts, and cost for the current billing window.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let usage = client.billing().usage().await?;
    /// println!("Total tokens: {}", usage.total_tokens);
    /// println!("Total requests: {}", usage.total_requests);
    /// println!("Total cost (cents): {}", usage.total_cost_cents);
    /// ```
    pub async fn usage(&self) -> Result<CustomerMeUsage> {
        let path = "/customers/me/usage";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        let response: CustomerMeUsageResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(response.usage)
    }

    /// Get the authenticated customer's credit balance.
    ///
    /// For PAYGO (pay-as-you-go) subscriptions, returns the current balance
    /// and reserved amount.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let balance = client.billing().balance().await?;
    /// println!("Balance: {} cents", balance.balance_cents);
    /// println!("Reserved: {} cents", balance.reserved_cents);
    /// ```
    pub async fn balance(&self) -> Result<CustomerBalanceResponse> {
        let path = "/customers/me/balance";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        self.inner
            .execute_json(builder, Method::GET, None, ctx)
            .await
    }

    /// Get the authenticated customer's balance transaction history.
    ///
    /// Returns a list of ledger entries showing credits and debits.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let history = client.billing().balance_history().await?;
    /// for entry in history.entries {
    ///     println!("{}: {} cents ({})", entry.occurred_at, entry.amount_cents, entry.reason);
    /// }
    /// ```
    pub async fn balance_history(&self) -> Result<CustomerLedgerResponse> {
        let path = "/customers/me/balance/history";
        let builder = self.inner.request(Method::GET, path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, path, None, None);
        self.inner
            .execute_json(builder, Method::GET, None, ctx)
            .await
    }

    /// Create a top-up checkout session.
    ///
    /// For PAYGO subscriptions, creates a Stripe Checkout session to add credits.
    ///
    /// # Arguments
    ///
    /// * `req` - Top-up request with amount and redirect URLs
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let session = client.billing().topup(CustomerTopupRequest {
    ///     credit_amount_cents: 1000, // $10.00
    ///     success_url: "https://myapp.com/billing/success".into(),
    ///     cancel_url: "https://myapp.com/billing/cancel".into(),
    /// }).await?;
    /// println!("Checkout URL: {}", session.checkout_url);
    /// ```
    pub async fn topup(&self, req: CustomerTopupRequest) -> Result<CustomerTopupResponse> {
        let path = "/customers/me/topup";
        let mut builder = self.inner.request(Method::POST, path)?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, path, None, None);
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }

    /// Change the authenticated customer's subscription tier.
    ///
    /// Switches to a different tier within the same project.
    ///
    /// # Arguments
    ///
    /// * `tier_code` - The tier code to switch to
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let sub = client.billing().change_tier("pro").await?;
    /// println!("New tier: {:?}", sub.tier_code);
    /// ```
    pub async fn change_tier(&self, tier_code: &str) -> Result<CustomerMeSubscription> {
        let path = "/customers/me/change-tier";
        let req = ChangeTierRequest {
            tier_code: tier_code.to_string(),
        };
        let mut builder = self.inner.request(Method::POST, path)?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, path, None, None);
        let response: CustomerMeSubscriptionResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(response.subscription)
    }

    /// Create a subscription checkout session.
    ///
    /// Creates a Stripe Checkout session for subscribing to a tier.
    ///
    /// # Arguments
    ///
    /// * `req` - Checkout request with tier and redirect URLs
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let session = client.billing().checkout(CustomerMeCheckoutRequest {
    ///     tier_code: "pro".into(),
    ///     success_url: "https://myapp.com/billing/success".into(),
    ///     cancel_url: "https://myapp.com/billing/cancel".into(),
    /// }).await?;
    /// println!("Checkout URL: {}", session.url);
    /// ```
    pub async fn checkout(
        &self,
        req: CustomerMeCheckoutRequest,
    ) -> Result<CheckoutSessionResponse> {
        let path = "/customers/me/checkout";
        let mut builder = self.inner.request(Method::POST, path)?;
        builder = builder.json(&req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, path, None, None);
        self.inner
            .execute_json(builder, Method::POST, None, ctx)
            .await
    }
}
