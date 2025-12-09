#![cfg(feature = "mock")]

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::{
    errors::{Error, Result},
    types::{
        APIKey, FrontendToken, FrontendTokenRequest, MessageRole, Model, ProxyMessage, ProxyRequest, ProxyResponse,
        StreamEvent, TokenType, Usage,
    },
    ProxyOptions,
};

#[cfg(all(feature = "client", feature = "streaming"))]
use crate::ChatStreamAdapter;
#[cfg(feature = "streaming")]
use crate::{sse::StreamHandle, StreamEventKind};
#[cfg(feature = "blocking")]
use crate::{BlockingProxyHandle, ProxyOptions as BlockingProxyOptions};
use time::OffsetDateTime;
use uuid::Uuid;

/// In-memory mock configuration for offline tests.
#[derive(Default)]
pub struct MockConfig {
    pub proxy_responses: Vec<Result<ProxyResponse>>,
    pub stream_sequences: Vec<Vec<Result<StreamEvent>>>,
    pub frontend_tokens: Vec<Result<FrontendToken>>,
}

impl MockConfig {
    pub fn with_proxy_response(mut self, resp: ProxyResponse) -> Self {
        self.proxy_responses.push(Ok(resp));
        self
    }

    pub fn with_proxy_error(mut self, err: Error) -> Self {
        self.proxy_responses.push(Err(err));
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

    pub fn with_frontend_token(mut self, token: FrontendToken) -> Self {
        self.frontend_tokens.push(Ok(token));
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

    pub fn llm(&self) -> MockLLMClient {
        MockLLMClient {
            inner: self.inner.clone(),
        }
    }

    #[cfg(feature = "blocking")]
    pub fn blocking_llm(&self) -> MockBlockingLLMClient {
        MockBlockingLLMClient {
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
    proxy_responses: Mutex<VecDeque<Result<ProxyResponse>>>,
    stream_sequences: Mutex<VecDeque<Vec<Result<StreamEvent>>>>,
    frontend_tokens: Mutex<VecDeque<Result<FrontendToken>>>,
}

impl MockInner {
    fn new(cfg: MockConfig) -> Self {
        Self {
            proxy_responses: Mutex::new(VecDeque::from(cfg.proxy_responses)),
            stream_sequences: Mutex::new(VecDeque::from(cfg.stream_sequences)),
            frontend_tokens: Mutex::new(VecDeque::from(cfg.frontend_tokens)),
        }
    }

    fn next_proxy(&self) -> Result<ProxyResponse> {
        self.proxy_responses
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .unwrap_or_else(|| Err(Error::Validation("no mock proxy response queued".into())))
    }

    #[cfg(feature = "streaming")]
    fn next_stream(&self) -> Result<Vec<Result<StreamEvent>>> {
        self.stream_sequences
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .ok_or_else(|| Error::Validation("no mock stream events queued".into()))
    }

    fn next_frontend_token(&self) -> Result<FrontendToken> {
        self.frontend_tokens
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .unwrap_or_else(|| Err(Error::Validation("no mock frontend token queued".into())))
    }
}

#[derive(Clone)]
pub struct MockLLMClient {
    inner: Arc<MockInner>,
}

#[derive(Clone)]
pub struct MockAuthClient {
    inner: Arc<MockInner>,
}

impl MockAuthClient {
    pub async fn frontend_token(&self, _req: FrontendTokenRequest) -> Result<FrontendToken> {
        self.inner.next_frontend_token()
    }
}

impl MockLLMClient {
    pub async fn proxy(&self, req: ProxyRequest, options: ProxyOptions) -> Result<ProxyResponse> {
        req.validate()?;
        let mut resp = self.inner.next_proxy()?;
        if resp.request_id.is_none() {
            resp.request_id = options.request_id;
        }
        Ok(resp)
    }

    #[cfg(feature = "streaming")]
    pub async fn proxy_stream(
        &self,
        req: ProxyRequest,
        options: ProxyOptions,
    ) -> Result<StreamHandle> {
        req.validate()?;
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

    #[cfg(all(feature = "client", feature = "streaming"))]
    pub async fn proxy_stream_deltas(
        &self,
        req: ProxyRequest,
        options: ProxyOptions,
    ) -> Result<std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<String>> + Send>>> {
        let stream = self.proxy_stream(req, options).await?;
        Ok(Box::pin(
            ChatStreamAdapter::<crate::StreamHandle>::new(stream).into_stream(),
        ))
    }
}

#[cfg(feature = "blocking")]
#[derive(Clone)]
pub struct MockBlockingLLMClient {
    inner: Arc<MockInner>,
}

#[cfg(feature = "blocking")]
impl MockBlockingLLMClient {
    pub fn proxy(&self, req: ProxyRequest, options: BlockingProxyOptions) -> Result<ProxyResponse> {
        req.validate()?;
        let mut resp = self.inner.next_proxy()?;
        if resp.request_id.is_none() {
            resp.request_id = options.request_id;
        }
        Ok(resp)
    }

    #[cfg(feature = "streaming")]
    pub fn proxy_stream(
        &self,
        req: ProxyRequest,
        options: BlockingProxyOptions,
    ) -> Result<BlockingProxyHandle> {
        req.validate()?;
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
        Ok(BlockingProxyHandle::from_events_with_request_id(
            events, req_id,
        ))
    }

    #[cfg(all(feature = "blocking", feature = "streaming"))]
    pub fn proxy_stream_deltas(
        &self,
        req: ProxyRequest,
        options: BlockingProxyOptions,
    ) -> Result<Box<dyn Iterator<Item = Result<String>>>> {
        let stream = self.proxy_stream(req, options)?;
        Ok(Box::new(
            crate::ChatStreamAdapter::<crate::BlockingProxyHandle>::new(stream).into_iter(),
        ))
    }
}

pub mod fixtures {
    use super::*;

    pub fn simple_proxy_response() -> ProxyResponse {
        ProxyResponse {
            id: "resp_mock_123".into(),
            content: vec!["hello world".into()],
            stop_reason: Some(crate::StopReason::Stop),
            model: Model::from("gpt-4o-mini"),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
            },
            request_id: Some("req_mock_123".into()),
            tool_calls: None,
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

    pub fn frontend_token() -> FrontendToken {
        FrontendToken {
            token: "mr_ft_mock".into(),
            expires_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
            expires_in: 3600,
            token_type: TokenType::Bearer,
            key_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            customer_id: Uuid::new_v4(),
            customer_external_id: "cust_mock_123".into(),
            tier_code: "free".into(),
            device_id: None,
            publishable_key: None,
        }
    }

    pub fn api_key(label: &str) -> APIKey {
        APIKey {
            id: Uuid::new_v4(),
            label: label.into(),
            kind: "secret".into(),
            created_at: OffsetDateTime::now_utc(),
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
    use crate::ProxyMessage;
    #[cfg(feature = "streaming")]
    use futures_util::StreamExt;

    #[tokio::test]
    async fn proxy_returns_queued_response() {
        let cfg = MockConfig::default().with_proxy_response(fixtures::simple_proxy_response());
        let client = MockClient::new(cfg);
        let resp = client
            .llm()
            .proxy(
                ProxyRequest::new(
                    Model::from("gpt-4o-mini"),
                    vec![ProxyMessage {
                        role: MessageRole::User,
                        content: "hi".into(),
                        tool_calls: None,
                        tool_call_id: None,
                    }],
                )
                .unwrap(),
                ProxyOptions::default().with_request_id("req_override"),
            )
            .await
            .unwrap();
        assert_eq!(resp.content.join(""), "hello world");
        assert_eq!(resp.request_id.as_deref(), Some("req_mock_123"));
    }

    #[cfg(feature = "streaming")]
    #[tokio::test]
    async fn proxy_stream_yields_events() {
        let cfg = MockConfig::default().with_stream_events(fixtures::simple_stream_events());
        let client = MockClient::new(cfg);
        let mut stream = client
            .llm()
            .proxy_stream(
                ProxyRequest::new(
                    Model::from("gpt-4o-mini"),
                    vec![ProxyMessage {
                        role: MessageRole::User,
                        content: "hi".into(),
                        tool_calls: None,
                        tool_call_id: None,
                    }],
                )
                .unwrap(),
                ProxyOptions::default(),
            )
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

    #[cfg(all(feature = "client", feature = "streaming"))]
    #[tokio::test]
    async fn proxy_stream_delta_adapter_collects() {
        use futures_util::StreamExt;

        let cfg = MockConfig::default().with_stream_events(fixtures::simple_stream_events());
        let client = MockClient::new(cfg);
        let mut deltas = String::new();
        let stream = client
            .llm()
            .proxy_stream_deltas(
                ProxyRequest::new(
                    Model::from("gpt-4o-mini"),
                    vec![ProxyMessage {
                        role: MessageRole::User,
                        content: "hi".into(),
                        tool_calls: None,
                        tool_call_id: None,
                    }],
                )
                .unwrap(),
                ProxyOptions::default(),
            )
            .await
            .unwrap();
        futures_util::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            deltas.push_str(&chunk.unwrap());
        }
        assert_eq!(deltas, "hello");
    }

    #[tokio::test]
    async fn proxy_builder_smoke_test() {
        let mut resp = fixtures::simple_proxy_response();
        resp.request_id = None;
        let cfg = MockConfig::default().with_proxy_response(resp);
        let client = MockClient::new(cfg);
        let req = ProxyRequest::builder(Model::from("openai/gpt-4o-mini"))
            .user("hi")
            .build()
            .unwrap();

        let resp = client
            .llm()
            .proxy(req, ProxyOptions::default().with_request_id("req_builder"))
            .await
            .unwrap();
        assert_eq!(resp.id, "resp_mock_123");
        assert_eq!(resp.request_id.as_deref(), Some("req_builder"));
    }

    #[cfg(feature = "blocking")]
    #[test]
    fn blocking_proxy_returns_response() {
        let cfg = MockConfig::default().with_proxy_response(fixtures::simple_proxy_response());
        let client = MockClient::new(cfg);
        let resp = client
            .blocking_llm()
            .proxy(
                ProxyRequest::new(
                    Model::from("openai/gpt-4o-mini"),
                    vec![ProxyMessage {
                        role: MessageRole::User,
                        content: "hi".into(),
                        tool_calls: None,
                        tool_call_id: None,
                    }],
                )
                .unwrap(),
                BlockingProxyOptions::default(),
            )
            .unwrap();
        assert_eq!(resp.model, Model::from("gpt-4o-mini"));
    }
}
