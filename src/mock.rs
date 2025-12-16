// The mock module's low-level proxy methods are pub(crate) because the public API
// only exposes builders. This module can be used for internal testing or redesigned
// to work with builders in the future.
#![allow(dead_code)]

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::{
    errors::{Error, Result},
    generated,
    types::{
        APIKey, CustomerToken, CustomerTokenRequest, DeviceStartRequest, DeviceTokenResult, Model,
        Response, ResponseRequest, StreamEvent, Usage,
    },
    ResponseOptions,
};

#[cfg(feature = "streaming")]
use crate::ResponseStreamAdapter;
#[cfg(feature = "streaming")]
use crate::{ndjson::StreamHandle, StreamEventKind};
#[cfg(feature = "blocking")]
use crate::{BlockingStreamHandle, ResponseOptions as BlockingResponseOptions};
use chrono::{Duration, Utc};
use uuid::Uuid;

/// In-memory mock configuration for offline tests.
#[derive(Default)]
pub struct MockConfig {
    pub responses: Vec<Result<Response>>,
    pub stream_sequences: Vec<Vec<Result<StreamEvent>>>,
    pub customer_tokens: Vec<Result<CustomerToken>>,
}

impl MockConfig {
    pub fn with_response(mut self, resp: Response) -> Self {
        self.responses.push(Ok(resp));
        self
    }

    pub fn with_response_error(mut self, err: Error) -> Self {
        self.responses.push(Err(err));
        self
    }

    pub fn with_stream_events(mut self, events: Vec<StreamEvent>) -> Self {
        self.stream_sequences
            .push(events.into_iter().map(Ok).collect());
        self
    }

    pub fn with_stream_results(mut self, events: Vec<Result<StreamEvent>>) -> Self {
        self.stream_sequences.push(events);
        self
    }

    pub fn with_customer_token(mut self, token: CustomerToken) -> Self {
        self.customer_tokens.push(Ok(token));
        self
    }
}

#[derive(Clone)]
pub struct MockClient {
    inner: Arc<MockInner>,
}

impl MockClient {
    pub fn new(cfg: MockConfig) -> Self {
        Self {
            inner: Arc::new(MockInner::new(cfg)),
        }
    }

    pub fn responses(&self) -> MockResponsesClient {
        MockResponsesClient {
            inner: self.inner.clone(),
        }
    }

    #[cfg(feature = "blocking")]
    pub fn blocking_responses(&self) -> MockBlockingResponsesClient {
        MockBlockingResponsesClient {
            inner: self.inner.clone(),
        }
    }

    pub fn auth(&self) -> MockAuthClient {
        MockAuthClient {
            inner: self.inner.clone(),
        }
    }
}

struct MockInner {
    responses: Mutex<VecDeque<Result<Response>>>,
    stream_sequences: Mutex<VecDeque<Vec<Result<StreamEvent>>>>,
    customer_tokens: Mutex<VecDeque<Result<CustomerToken>>>,
}

impl MockInner {
    fn new(cfg: MockConfig) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(cfg.responses)),
            stream_sequences: Mutex::new(VecDeque::from(cfg.stream_sequences)),
            customer_tokens: Mutex::new(VecDeque::from(cfg.customer_tokens)),
        }
    }

    fn next_response(&self) -> Result<Response> {
        self.responses
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .unwrap_or_else(|| Err(Error::Validation("no mock response queued".into())))
    }

    #[cfg(feature = "streaming")]
    fn next_stream(&self) -> Result<Vec<Result<StreamEvent>>> {
        self.stream_sequences
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .ok_or_else(|| Error::Validation("no mock stream events queued".into()))
    }

    fn next_customer_token(&self) -> Result<CustomerToken> {
        self.customer_tokens
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .unwrap_or_else(|| Err(Error::Validation("no mock customer token queued".into())))
    }
}

#[derive(Clone)]
pub struct MockResponsesClient {
    inner: Arc<MockInner>,
}

#[derive(Clone)]
pub struct MockAuthClient {
    inner: Arc<MockInner>,
}

impl MockAuthClient {
    pub async fn customer_token(&self, _req: CustomerTokenRequest) -> Result<CustomerToken> {
        self.inner.next_customer_token()
    }

    /// Mock device_start that returns a test response.
    pub async fn device_start(
        &self,
        _req: DeviceStartRequest,
    ) -> Result<generated::DeviceStartResponse> {
        Ok(generated::DeviceStartResponse {
            device_code: "mock_device_code".into(),
            user_code: "MOCK-CODE".into(),
            verification_uri: "https://example.com/device".into(),
            verification_uri_complete: Some("https://example.com/device?code=MOCK-CODE".into()),
            expires_in: 900,
            interval: 5,
        })
    }

    /// Mock device_token that returns an error (not yet mocked).
    pub async fn device_token(&self, _device_code: &str) -> Result<DeviceTokenResult> {
        Err(Error::Validation("device flow not mocked".into()))
    }
}

impl MockResponsesClient {
    pub(crate) async fn create(
        &self,
        req: ResponseRequest,
        options: ResponseOptions,
    ) -> Result<Response> {
        req.validate(true)?;
        let mut resp = self.inner.next_response()?;
        if resp.request_id.is_none() {
            resp.request_id = options.request_id;
        }
        Ok(resp)
    }

    #[cfg(feature = "streaming")]
    pub(crate) async fn stream(
        &self,
        req: ResponseRequest,
        options: ResponseOptions,
    ) -> Result<StreamHandle> {
        req.validate(true)?;
        let results = self.inner.next_stream()?;
        let mut events = Vec::new();
        for res in results {
            events.push(res?);
        }
        let req_id = options.request_id.clone().or_else(|| {
            events
                .iter()
                .find_map(|evt| evt.request_id.clone().filter(|v| !v.is_empty()))
        });
        let events = events
            .into_iter()
            .map(|mut evt| {
                if evt.request_id.is_none() {
                    evt.request_id = req_id.clone();
                }
                evt
            })
            .collect::<Vec<_>>();
        Ok(StreamHandle::from_events_with_request_id(events, req_id))
    }

    #[cfg(feature = "streaming")]
    pub(crate) async fn stream_deltas(
        &self,
        req: ResponseRequest,
        options: ResponseOptions,
    ) -> Result<std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<String>> + Send>>> {
        let stream = self.stream(req, options).await?;
        Ok(Box::pin(
            ResponseStreamAdapter::<crate::StreamHandle>::new(stream).into_stream(),
        ))
    }
}

#[cfg(feature = "blocking")]
#[derive(Clone)]
pub struct MockBlockingResponsesClient {
    inner: Arc<MockInner>,
}

#[cfg(feature = "blocking")]
impl MockBlockingResponsesClient {
    pub(crate) fn create(
        &self,
        req: ResponseRequest,
        options: BlockingResponseOptions,
    ) -> Result<Response> {
        req.validate(true)?;
        let mut resp = self.inner.next_response()?;
        if resp.request_id.is_none() {
            resp.request_id = options.request_id;
        }
        Ok(resp)
    }

    #[cfg(feature = "streaming")]
    pub(crate) fn stream(
        &self,
        req: ResponseRequest,
        options: BlockingResponseOptions,
    ) -> Result<BlockingStreamHandle> {
        req.validate(true)?;
        let results = self.inner.next_stream()?;
        let mut events = Vec::new();
        for res in results {
            events.push(res?);
        }
        let req_id = options.request_id.clone().or_else(|| {
            events
                .iter()
                .find_map(|evt| evt.request_id.clone().filter(|v| !v.is_empty()))
        });
        let events = events
            .into_iter()
            .map(|mut evt| {
                if evt.request_id.is_none() {
                    evt.request_id = req_id.clone();
                }
                evt
            })
            .collect::<Vec<_>>();
        Ok(BlockingStreamHandle::from_events_with_request_id(
            events, req_id,
        ))
    }

    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub(crate) fn stream_deltas(
        &self,
        req: ResponseRequest,
        options: BlockingResponseOptions,
    ) -> Result<Box<dyn Iterator<Item = Result<String>>>> {
        let stream = self.stream(req, options)?;
        Ok(Box::new(
            crate::ResponseStreamAdapter::<crate::BlockingStreamHandle>::new(stream).into_iter(),
        ))
    }
}

pub mod fixtures {
    use super::*;

    pub fn simple_response() -> Response {
        Response {
            id: "resp_mock_123".into(),
            stop_reason: Some(crate::StopReason::Stop),
            model: Model::from("gpt-4o-mini"),
            output: vec![crate::OutputItem::Message {
                role: crate::MessageRole::Assistant,
                content: vec![crate::ContentPart::text("hello world")],
                tool_calls: None,
            }],
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
            },
            request_id: Some("req_mock_123".into()),
            provider: None,
            citations: None,
        }
    }

    #[cfg(feature = "streaming")]
    pub fn simple_stream_events() -> Vec<StreamEvent> {
        vec![
            StreamEvent {
                kind: StreamEventKind::MessageStart,
                event: "message_start".into(),
                data: None,
                text_delta: None,
                tool_call_delta: None,
                tool_calls: None,
                response_id: Some("resp_stream_mock".into()),
                model: Some(Model::from("gpt-4o-mini")),
                stop_reason: None,
                usage: None,
                request_id: Some("req_stream_mock".into()),
                raw: "{}".into(),
            },
            StreamEvent {
                kind: StreamEventKind::MessageDelta,
                event: "message_delta".into(),
                data: None,
                text_delta: Some("hello".into()),
                tool_call_delta: None,
                tool_calls: None,
                response_id: Some("resp_stream_mock".into()),
                model: Some(Model::from("gpt-4o-mini")),
                stop_reason: None,
                usage: None,
                request_id: Some("req_stream_mock".into()),
                raw: "{}".into(),
            },
            StreamEvent {
                kind: StreamEventKind::MessageStop,
                event: "message_stop".into(),
                data: None,
                text_delta: None,
                tool_call_delta: None,
                tool_calls: None,
                response_id: Some("resp_stream_mock".into()),
                model: Some(Model::from("gpt-4o-mini")),
                stop_reason: Some(crate::StopReason::Completed),
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                }),
                request_id: Some("req_stream_mock".into()),
                raw: "{}".into(),
            },
        ]
    }

    pub fn customer_token() -> CustomerToken {
        CustomerToken {
            token: "mr_ct_mock".into(),
            expires_at: Utc::now() + Duration::hours(1),
            expires_in: 3600,
            project_id: Uuid::new_v4(),
            customer_id: Uuid::new_v4(),
            customer_external_id: "cust_mock_123".into(),
            tier_code: "free".parse().expect("valid tier code"),
        }
    }

    pub fn api_key(label: &str) -> APIKey {
        APIKey {
            id: Uuid::new_v4(),
            label: label.into(),
            kind: "secret".into(),
            created_at: Utc::now(),
            expires_at: None,
            last_used_at: None,
            redacted_key: format!("mr_sk_{label}_redacted"),
            secret_key: Some(format!("mr_sk_{label}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "streaming")]
    use futures_util::StreamExt;

    /// Helper to create a simple ResponseRequest for testing.
    fn simple_request(model: &str) -> ResponseRequest {
        ResponseRequest {
            provider: None,
            model: Some(Model::from(model)),
            input: vec![crate::InputItem::user("hi")],
            output_format: None,
            max_output_tokens: None,
            temperature: None,
            stop: None,
            tools: None,
            tool_choice: None,
        }
    }

    #[tokio::test]
    async fn responses_returns_queued_response() {
        let cfg = MockConfig::default().with_response(fixtures::simple_response());
        let client = MockClient::new(cfg);
        let resp = client
            .responses()
            .create(
                simple_request("gpt-4o-mini"),
                ResponseOptions::default().with_request_id("req_override"),
            )
            .await
            .unwrap();
        let text = resp
            .output
            .iter()
            .filter_map(|item| match item {
                crate::OutputItem::Message { role, content, .. } => {
                    (*role == crate::MessageRole::Assistant).then_some(content)
                }
            })
            .flatten()
            .filter_map(|p| match p {
                crate::ContentPart::Text { text } => Some(text.as_str()),
            })
            .collect::<String>();
        assert_eq!(text, "hello world");
        assert_eq!(resp.request_id.as_deref(), Some("req_mock_123"));
    }

    #[cfg(feature = "streaming")]
    #[tokio::test]
    async fn responses_stream_yields_events() {
        let cfg = MockConfig::default().with_stream_events(fixtures::simple_stream_events());
        let client = MockClient::new(cfg);
        let mut stream = client
            .responses()
            .stream(simple_request("gpt-4o-mini"), ResponseOptions::default())
            .await
            .unwrap();

        let mut deltas = String::new();
        while let Some(evt) = stream.next().await {
            let evt = evt.unwrap();
            if let Some(text) = evt.text_delta {
                deltas.push_str(&text);
            }
        }
        assert_eq!(deltas, "hello");
    }

    #[cfg(feature = "streaming")]
    #[tokio::test]
    async fn responses_stream_delta_adapter_collects() {
        use futures_util::StreamExt;

        let cfg = MockConfig::default().with_stream_events(fixtures::simple_stream_events());
        let client = MockClient::new(cfg);
        let mut deltas = String::new();
        let stream = client
            .responses()
            .stream_deltas(simple_request("gpt-4o-mini"), ResponseOptions::default())
            .await
            .unwrap();
        futures_util::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            deltas.push_str(&chunk.unwrap());
        }
        assert_eq!(deltas, "hello");
    }

    #[tokio::test]
    async fn responses_smoke_test() {
        let mut resp = fixtures::simple_response();
        resp.request_id = None;
        let cfg = MockConfig::default().with_response(resp);
        let client = MockClient::new(cfg);

        let resp = client
            .responses()
            .create(
                simple_request("openai/gpt-4o-mini"),
                ResponseOptions::default().with_request_id("req_test"),
            )
            .await
            .unwrap();
        assert_eq!(resp.id, "resp_mock_123");
        assert_eq!(resp.request_id.as_deref(), Some("req_test"));
    }

    #[cfg(feature = "blocking")]
    #[test]
    fn blocking_responses_returns_response() {
        let cfg = MockConfig::default().with_response(fixtures::simple_response());
        let client = MockClient::new(cfg);
        let resp = client
            .blocking_responses()
            .create(
                simple_request("openai/gpt-4o-mini"),
                BlockingResponseOptions::default(),
            )
            .unwrap();
        assert_eq!(resp.model, Model::from("gpt-4o-mini"));
    }
}
