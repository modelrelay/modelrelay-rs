//! Sessions API client for multi-turn conversation management.
//!
//! Sessions provide stateful multi-turn conversation management. They persist
//! message history on the server for cross-device continuity, team collaboration,
//! and audit trails.
//!
//! # Example
//!
//! ```rust,ignore
//! use modelrelay::Client;
//!
//! let client = Client::from_secret_key(std::env::var("MODELRELAY_API_KEY")?)?
//!     .build()?;
//!
//! // Create a session
//! let session = client.sessions().create(SessionCreateRequest {
//!     customer_id: None,
//!     metadata: Some(serde_json::json!({"project": "my-app"})),
//! }).await?;
//!
//! // Get session with messages
//! let session_with_messages = client.sessions().get(&session.id.to_string()).await?;
//!
//! // Add a message
//! let message = client.sessions().add_message(&session.id.to_string(), SessionMessageCreateRequest {
//!     role: "user".into(),
//!     content: vec![serde_json::json!({"type": "text", "text": "Hello!"})],
//!     run_id: None,
//! }).await?;
//! ```

use std::sync::Arc;

use reqwest::Method;

use crate::{
    client::ClientInner,
    errors::{Result, ValidationError},
    generated::{
        SessionCreateRequest, SessionListResponse, SessionMessageCreateRequest,
        SessionMessageResponse, SessionResponse, SessionWithMessagesResponse,
    },
    http::HeaderList,
    Error,
};

/// Client for session operations.
///
/// Sessions enable multi-turn conversations with persistent history stored on the server.
#[derive(Clone)]
pub struct SessionsClient {
    pub(crate) inner: Arc<ClientInner>,
}

/// Options for listing sessions.
#[derive(Debug, Default, Clone)]
pub struct ListSessionsOptions {
    /// Maximum number of sessions to return (default: 50, max: 100).
    pub limit: Option<i32>,
    /// Number of sessions to skip (for pagination).
    pub offset: Option<i32>,
    /// Pagination cursor from a previous response's next_cursor field.
    pub cursor: Option<String>,
    /// Filter sessions by customer ID.
    pub customer_id: Option<String>,
}

impl SessionsClient {
    /// Create a new session.
    ///
    /// Sessions are project-scoped and can optionally be associated with a customer.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let session = client.sessions().create(SessionCreateRequest {
    ///     customer_id: Some(uuid::Uuid::parse_str("...")?),
    ///     metadata: Some(serde_json::json!({"project": "my-app"})),
    /// }).await?;
    /// println!("Created session: {}", session.id);
    /// ```
    pub async fn create(&self, req: SessionCreateRequest) -> Result<SessionResponse> {
        let path = "/sessions";
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
        let resp: SessionResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }

    /// List sessions with pagination.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let resp = client.sessions().list(ListSessionsOptions {
    ///     limit: Some(10),
    ///     ..Default::default()
    /// }).await?;
    ///
    /// for session in resp.sessions {
    ///     println!("Session {}: {} messages", session.id, session.message_count);
    /// }
    ///
    /// // Fetch next page using cursor
    /// if let Some(cursor) = resp.next_cursor {
    ///     let next_page = client.sessions().list(ListSessionsOptions {
    ///         cursor: Some(cursor),
    ///         ..Default::default()
    ///     }).await?;
    /// }
    /// ```
    pub async fn list(&self, opts: ListSessionsOptions) -> Result<SessionListResponse> {
        let mut path = "/sessions".to_string();
        let mut params: Vec<(&str, String)> = Vec::new();

        if let Some(limit) = opts.limit {
            params.push(("limit", limit.to_string()));
        }
        if let Some(offset) = opts.offset {
            params.push(("offset", offset.to_string()));
        }
        if let Some(ref cursor) = opts.cursor {
            params.push(("cursor", cursor.clone()));
        }
        if let Some(ref customer_id) = opts.customer_id {
            params.push(("customer_id", customer_id.clone()));
        }

        if !params.is_empty() {
            let encoded: String = params
                .iter()
                .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            path = format!("{}?{}", path, encoded);
        }

        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp: SessionListResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp)
    }

    /// Get a session by ID, including its full message history.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let session = client.sessions().get("session-uuid").await?;
    /// for msg in session.messages {
    ///     println!("[{}] {:?}", msg.role, msg.content);
    /// }
    /// ```
    pub async fn get(&self, session_id: &str) -> Result<SessionWithMessagesResponse> {
        if session_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("session_id is required").with_field("session_id"),
            ));
        }

        let path = format!("/sessions/{}", session_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp: SessionWithMessagesResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp)
    }

    /// Delete a session.
    ///
    /// Requires a secret key.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// client.sessions().delete("session-uuid").await?;
    /// ```
    pub async fn delete(&self, session_id: &str) -> Result<()> {
        if session_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("session_id is required").with_field("session_id"),
            ));
        }

        let path = format!("/sessions/{}", session_id);
        let builder = self.inner.request(Method::DELETE, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let retry_cfg = self.inner.retry.clone();
        let ctx = self.inner.make_context(&Method::DELETE, &path, None, None);
        let _ = self
            .inner
            .send_with_retry(builder, Method::DELETE, retry_cfg, ctx)
            .await?;
        Ok(())
    }

    /// Add a message to a session.
    ///
    /// Messages can be user, assistant, or tool messages. Assistant messages
    /// can optionally include a run_id to link them to a workflow run.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let msg = client.sessions().add_message("session-uuid", SessionMessageCreateRequest {
    ///     role: "user".into(),
    ///     content: vec![serde_json::json!({"type": "text", "text": "Hello!"})],
    ///     run_id: None,
    /// }).await?;
    /// println!("Added message: {}", msg.id);
    /// ```
    pub async fn add_message(
        &self,
        session_id: &str,
        req: SessionMessageCreateRequest,
    ) -> Result<SessionMessageResponse> {
        if session_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("session_id is required").with_field("session_id"),
            ));
        }

        let role = req.role.trim();
        if role.is_empty() {
            return Err(Error::Validation(
                ValidationError::new("role is required").with_field("role"),
            ));
        }
        match role {
            "user" | "assistant" | "tool" => {}
            _ => {
                return Err(Error::Validation(
                    ValidationError::new(format!(
                        "invalid role '{}' (must be user, assistant, or tool)",
                        role
                    ))
                    .with_field("role"),
                ));
            }
        }

        if req.content.is_empty() {
            return Err(Error::Validation(
                ValidationError::new("content is required").with_field("content"),
            ));
        }

        // Use trimmed role in request to normalize whitespace
        let normalized_req = SessionMessageCreateRequest {
            role: role.to_string(),
            content: req.content,
            run_id: req.run_id,
        };

        let path = format!("/sessions/{}/messages", session_id);
        let mut builder = self.inner.request(Method::POST, &path)?;
        builder = builder.json(&normalized_req);
        let builder = self.inner.with_headers(
            builder,
            None,
            &HeaderList::default(),
            Some("application/json"),
        )?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, &path, None, None);
        let resp: SessionMessageResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }
}
