//! Image generation API client.

use std::sync::Arc;

use reqwest::Method;

use crate::{
    client::ClientInner,
    errors::{Result, ValidationError},
    generated::{ImageRequest, ImageResponse},
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
}
