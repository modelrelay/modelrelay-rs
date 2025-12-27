//! Image generation API client.

use std::sync::Arc;

use reqwest::Method;

use crate::{
    client::ClientInner,
    errors::{Result, ValidationError},
    generated::{ImagePinResponse, ImageRequest, ImageResponse},
    http::HeaderList,
    Error,
};

/// Client for image generation operations.
///
/// # Example
///
/// ```rust,ignore
/// use modelrelay::{Client, ImageRequest, ImageResponseFormat};
///
/// // Production use (default) - returns URLs
/// let response = client.images().generate(ImageRequest {
///     model: "gemini-2.5-flash-image".into(),
///     prompt: "A futuristic cityscape".into(),
///     response_format: None, // defaults to URL
/// }).await?;
///
/// println!("{}", response.data[0].url.as_deref().unwrap_or_default());
/// println!("{}", response.data[0].mime_type.as_deref().unwrap_or_default());
///
/// // Testing/development - returns base64
/// let response = client.images().generate(ImageRequest {
///     model: "gemini-2.5-flash-image".into(),
///     prompt: "A futuristic cityscape".into(),
///     response_format: Some(ImageResponseFormat::B64Json),
/// }).await?;
/// ```
#[derive(Clone)]
pub struct ImagesClient {
    pub(crate) inner: Arc<ClientInner>,
}

impl ImagesClient {
    /// Generate images from a text prompt.
    ///
    /// By default, returns URLs (requires storage configuration on the server).
    /// Use `response_format: Some(ImageResponseFormat::B64Json)` for testing without storage.
    ///
    /// Model is optional when using a customer token with a tier that defines a default model.
    pub async fn generate(&self, req: ImageRequest) -> Result<ImageResponse> {
        if req.prompt.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("prompt is required").with_field("prompt"),
            ));
        }

        let path = "/images/generate";
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
        let resp: ImageResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }

    /// Get information about a specific image.
    ///
    /// Returns the image's pinned status, expiration time, and URL.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let info = client.images().get("img_abc123").await?;
    /// println!("Pinned: {}", info.pinned);
    /// if let Some(expires) = &info.expires_at {
    ///     println!("Expires: {}", expires);
    /// }
    /// ```
    pub async fn get(&self, image_id: &str) -> Result<ImagePinResponse> {
        if image_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("image_id is required").with_field("image_id"),
            ));
        }

        let path = format!("/images/{}", image_id);
        let builder = self.inner.request(Method::GET, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::GET, &path, None, None);
        let resp: ImagePinResponse = self
            .inner
            .execute_json(builder, Method::GET, None, ctx)
            .await?;
        Ok(resp)
    }

    /// Pin an image to prevent it from expiring.
    ///
    /// Pinned images remain accessible permanently (subject to tier limits).
    /// Returns the updated image state including its permanent URL.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let pinned = client.images().pin("img_abc123").await?;
    /// assert!(pinned.pinned);
    /// assert!(pinned.expires_at.is_none()); // No expiration when pinned
    /// ```
    pub async fn pin(&self, image_id: &str) -> Result<ImagePinResponse> {
        if image_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("image_id is required").with_field("image_id"),
            ));
        }

        let path = format!("/images/{}/pin", image_id);
        let builder = self.inner.request(Method::POST, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::POST, &path, None, None);
        let resp: ImagePinResponse = self
            .inner
            .execute_json(builder, Method::POST, None, ctx)
            .await?;
        Ok(resp)
    }

    /// Unpin an image, allowing it to expire.
    ///
    /// The image will be set to expire after the default ephemeral period (7 days).
    /// Returns the updated image state including the new expiration time.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let unpinned = client.images().unpin("img_abc123").await?;
    /// assert!(!unpinned.pinned);
    /// assert!(unpinned.expires_at.is_some()); // Will expire in 7 days
    /// ```
    pub async fn unpin(&self, image_id: &str) -> Result<ImagePinResponse> {
        if image_id.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("image_id is required").with_field("image_id"),
            ));
        }

        let path = format!("/images/{}/pin", image_id);
        let builder = self.inner.request(Method::DELETE, &path)?;
        let builder = self
            .inner
            .with_headers(builder, None, &HeaderList::default(), None)?;
        let builder = self.inner.with_timeout(builder, None, true);
        let ctx = self.inner.make_context(&Method::DELETE, &path, None, None);
        let resp: ImagePinResponse = self
            .inner
            .execute_json(builder, Method::DELETE, None, ctx)
            .await?;
        Ok(resp)
    }
}
