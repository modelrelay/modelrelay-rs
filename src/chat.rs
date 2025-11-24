#![cfg(any(feature = "client", feature = "blocking"))]

use std::collections::HashMap;
use std::time::Duration;

use crate::errors::{Error, Result};
use crate::types::{ProxyMessage, ProxyRequest, ProxyResponse};
#[cfg(feature = "streaming")]
use crate::types::{StreamEventKind, Usage};

#[cfg(feature = "blocking")]
use crate::blocking::BlockingLLMClient;
#[cfg(all(feature = "blocking", feature = "streaming"))]
use crate::blocking::BlockingProxyHandle;
#[cfg(feature = "client")]
use crate::client::LLMClient;
#[cfg(all(feature = "client", feature = "streaming"))]
use crate::sse::StreamHandle;
#[cfg(any(feature = "client", feature = "blocking"))]
use crate::{ProxyOptions, RetryConfig};

/// Builder for LLM proxy chat requests (async + streaming).
#[derive(Clone, Debug, Default)]
pub struct ChatRequestBuilder {
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) max_tokens: Option<i64>,
    pub(crate) temperature: Option<f64>,
    pub(crate) messages: Vec<ProxyMessage>,
    pub(crate) metadata: Option<HashMap<String, String>>,
    pub(crate) stop: Option<Vec<String>>,
    pub(crate) stop_sequences: Option<Vec<String>>,
    pub(crate) request_id: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) retry: Option<RetryConfig>,
}

impl ChatRequestBuilder {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: Some(model.into()),
            ..Default::default()
        }
    }

    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn message(mut self, role: impl Into<String>, content: impl Into<String>) -> Self {
        self.messages.push(ProxyMessage {
            role: role.into(),
            content: content.into(),
        });
        self
    }

    pub fn messages(mut self, messages: Vec<ProxyMessage>) -> Self {
        self.messages = messages;
        self
    }

    pub fn max_tokens(mut self, max_tokens: i64) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn stop(mut self, stop: Vec<String>) -> Self {
        self.stop = Some(stop);
        self
    }

    pub fn stop_sequences(mut self, stop_sequences: Vec<String>) -> Self {
        self.stop_sequences = Some(stop_sequences);
        self
    }

    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn retry(mut self, retry: RetryConfig) -> Self {
        self.retry = Some(retry);
        self
    }

    fn build_options(&self) -> ProxyOptions {
        let mut opts = ProxyOptions::default();
        if let Some(req_id) = &self.request_id {
            opts = opts.with_request_id(req_id.clone());
        }
        for (k, v) in &self.headers {
            opts = opts.with_header(k.clone(), v.clone());
        }
        if let Some(timeout) = self.timeout {
            opts = opts.with_timeout(timeout);
        }
        if let Some(retry) = &self.retry {
            opts = opts.with_retry(retry.clone());
        }
        opts
    }

    fn build_request(&self) -> Result<ProxyRequest> {
        let model = self
            .model
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Config("model is required".into()))?;
        if self.messages.is_empty() {
            return Err(Error::Config("at least one message is required".into()));
        }
        Ok(ProxyRequest {
            provider: self.provider.clone(),
            model,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            messages: self.messages.clone(),
            metadata: self.metadata.clone(),
            stop: self.stop.clone(),
            stop_sequences: self.stop_sequences.clone(),
        })
    }

    /// Execute the chat request (non-streaming, async).
    #[cfg(feature = "client")]
    pub async fn send(self, client: &LLMClient) -> Result<ProxyResponse> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy(req, opts).await
    }

    /// Execute the chat request and stream responses (async).
    #[cfg(all(feature = "client", feature = "streaming"))]
    pub async fn stream(self, client: &LLMClient) -> Result<StreamHandle> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy_stream(req, opts).await
    }

    /// Execute the chat request (blocking).
    #[cfg(feature = "blocking")]
    pub fn send_blocking(self, client: &BlockingLLMClient) -> Result<ProxyResponse> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy(req, opts)
    }

    /// Execute the chat request and stream responses (blocking).
    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn stream_blocking(self, client: &BlockingLLMClient) -> Result<BlockingProxyHandle> {
        let req = self.build_request()?;
        let opts = self.build_options();
        client.proxy_stream(req, opts)
    }
}

/// Thin adapter over streaming events to yield text deltas and final metadata.
#[cfg(feature = "streaming")]
#[derive(Debug)]
pub struct ChatStreamAdapter<S> {
    inner: S,
    finished: bool,
    final_usage: Option<Usage>,
    final_stop_reason: Option<String>,
    final_request_id: Option<String>,
}

#[cfg(all(feature = "client", feature = "streaming"))]
impl ChatStreamAdapter<StreamHandle> {
    pub fn new(stream: StreamHandle) -> Self {
        Self {
            inner: stream,
            finished: false,
            final_usage: None,
            final_stop_reason: None,
            final_request_id: None,
        }
    }

    /// Pull the next text delta (if any) and track final usage/stop metadata.
    pub async fn next_delta(&mut self) -> Result<Option<String>> {
        use futures_util::StreamExt;

        while let Some(item) = self.inner.next().await {
            let evt = item?;
            match evt.kind {
                StreamEventKind::MessageDelta => {
                    if let Some(delta) = evt.text_delta {
                        return Ok(Some(delta));
                    }
                }
                StreamEventKind::MessageStop => {
                    self.finished = true;
                    self.final_usage = evt.usage;
                    self.final_stop_reason = evt.stop_reason;
                    self.final_request_id = evt
                        .request_id
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Final usage info if the stream finished.
    pub fn final_usage(&self) -> Option<&Usage> {
        self.final_usage.as_ref()
    }

    /// Final stop reason if the stream finished.
    pub fn final_stop_reason(&self) -> Option<&str> {
        self.final_stop_reason.as_deref()
    }

    /// Final request id if known.
    pub fn final_request_id(&self) -> Option<&str> {
        self.final_request_id.as_deref()
    }
}

/// Blocking streaming adapter.
#[cfg(all(feature = "blocking", feature = "streaming"))]
impl ChatStreamAdapter<BlockingProxyHandle> {
    pub fn new(stream: BlockingProxyHandle) -> Self {
        Self {
            inner: stream,
            finished: false,
            final_usage: None,
            final_stop_reason: None,
            final_request_id: None,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.inner.request_id()
    }

    pub fn next_delta(&mut self) -> Result<Option<String>> {
        while let Some(evt) = self.inner.next()? {
            match evt.kind {
                StreamEventKind::MessageDelta => {
                    if let Some(delta) = evt.text_delta {
                        return Ok(Some(delta));
                    }
                }
                StreamEventKind::MessageStop => {
                    self.finished = true;
                    self.final_usage = evt.usage;
                    self.final_stop_reason = evt.stop_reason;
                    self.final_request_id = evt
                        .request_id
                        .or_else(|| self.inner.request_id().map(|s| s.to_string()));
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub fn final_usage(&self) -> Option<&Usage> {
        self.final_usage.as_ref()
    }

    pub fn final_stop_reason(&self) -> Option<&str> {
        self.final_stop_reason.as_deref()
    }

    pub fn final_request_id(&self) -> Option<&str> {
        self.final_request_id.as_deref()
    }
}
