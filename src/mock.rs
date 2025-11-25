#![cfg(feature = "mock")]

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::{
    ProxyOptions,
    errors::{Error, Result, ValidationError},
    types::{
        APIKey, APIKeyCreateRequest, FrontendToken, FrontendTokenRequest, Model, Provider,
        ProxyRequest, ProxyResponse, StreamEvent, Usage,
    },
};

#[cfg(feature = "blocking")]
use crate::{BlockingProxyHandle, ProxyOptions as BlockingProxyOptions};
#[cfg(feature = "streaming")]
use crate::{StreamEventKind, sse::StreamHandle};
use time::OffsetDateTime;
use uuid::Uuid;

/// In-memory mock configuration for offline tests.
#[derive(Default)]
pub struct MockConfig {
    pub proxy_responses: Vec<Result<ProxyResponse>>,
    pub stream_sequences: Vec<Vec<Result<StreamEvent>>>,
    pub frontend_tokens: Vec<Result<FrontendToken>>,
    pub api_keys: Vec<APIKey>,
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

    pub fn with_api_keys(mut self, keys: Vec<APIKey>) -> Self {
        self.api_keys = keys;
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

    pub fn auth(&self) -> MockAuthClient {
        MockAuthClient {
            inner: self.inner.clone(),
        }
    }

    pub fn api_keys(&self) -> MockApiKeysClient {
        MockApiKeysClient {
            inner: self.inner.clone(),
        }
    }

    #[cfg(feature = "blocking")]
    pub fn blocking_llm(&self) -> MockBlockingLLMClient {
        MockBlockingLLMClient {
            inner: self.inner.clone(),
        }
    }
}

struct MockInner {
    proxy_responses: Mutex<VecDeque<Result<ProxyResponse>>>,
    stream_sequences: Mutex<VecDeque<Vec<Result<StreamEvent>>>>,
    frontend_tokens: Mutex<VecDeque<Result<FrontendToken>>>,
    api_keys: Mutex<Vec<APIKey>>,
}

impl MockInner {
    fn new(cfg: MockConfig) -> Self {
        Self {
            proxy_responses: Mutex::new(VecDeque::from(cfg.proxy_responses)),
            stream_sequences: Mutex::new(VecDeque::from(cfg.stream_sequences)),
            frontend_tokens: Mutex::new(VecDeque::from(cfg.frontend_tokens)),
            api_keys: Mutex::new(cfg.api_keys),
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

#[derive(Clone)]
pub struct MockApiKeysClient {
    inner: Arc<MockInner>,
}

impl MockApiKeysClient {
    pub async fn list(&self) -> Result<Vec<APIKey>> {
        Ok(self.inner.api_keys.lock().expect("lock poisoned").clone())
    }

    pub async fn create(&self, req: APIKeyCreateRequest) -> Result<APIKey> {
        if req.label.trim().is_empty() {
            return Err(Error::Validation(
                ValidationError::new("label is required").with_field("label"),
            ));
        }
        let mut api_keys = self.inner.api_keys.lock().expect("lock poisoned");
        let key = APIKey {
            id: Uuid::new_v4(),
            label: req.label,
            kind: "secret".into(),
            created_at: OffsetDateTime::now_utc(),
            expires_at: req.expires_at,
            last_used_at: None,
            redacted_key: "mr_sk_***".into(),
            secret_key: Some("mr_sk_mock".into()),
        };
        api_keys.push(key.clone());
        Ok(key)
    }

    pub async fn delete(&self, id: Uuid) -> Result<()> {
        if id.is_nil() {
            return Err(Error::Validation(
                ValidationError::new("id is required").with_field("id"),
            ));
        }
        let mut api_keys = self.inner.api_keys.lock().expect("lock poisoned");
        let original_len = api_keys.len();
        api_keys.retain(|k| k.id != id);
        if api_keys.len() == original_len {
            return Err(Error::Validation("api key not found".into()));
        }
        Ok(())
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
}

pub mod fixtures {
    use super::*;

    pub fn simple_proxy_response() -> ProxyResponse {
        ProxyResponse {
            provider: Provider::OpenAI,
            id: "resp_mock_123".into(),
            content: vec!["hello world".into()],
            stop_reason: Some(crate::StopReason::Stop),
            model: Model::OpenAIGpt4oMini,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
            },
            request_id: Some("req_mock_123".into()),
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
                response_id: Some("resp_stream_mock".into()),
                model: Some(Model::OpenAIGpt4oMini),
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
                response_id: Some("resp_stream_mock".into()),
                model: Some(Model::OpenAIGpt4oMini),
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
                response_id: Some("resp_stream_mock".into()),
                model: Some(Model::OpenAIGpt4oMini),
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
            expires_at: None,
            expires_in: Some(3600),
            token_type: Some("bearer".into()),
            key_id: None,
            session_id: None,
            token_scope: None,
            token_source: None,
            end_user_id: None,
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
                    Model::OpenAIGpt4oMini,
                    vec![ProxyMessage {
                        role: "user".into(),
                        content: "hi".into(),
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
                    Model::OpenAIGpt4oMini,
                    vec![ProxyMessage {
                        role: "user".into(),
                        content: "hi".into(),
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
        use crate::ChatStreamAdapter;
        use futures_util::StreamExt;

        let cfg = MockConfig::default().with_stream_events(fixtures::simple_stream_events());
        let client = MockClient::new(cfg);
        let stream = client
            .llm()
            .proxy_stream(
                ProxyRequest::new(
                    Model::OpenAIGpt4oMini,
                    vec![ProxyMessage {
                        role: "user".into(),
                        content: "hi".into(),
                    }],
                )
                .unwrap(),
                ProxyOptions::default(),
            )
            .await
            .unwrap();

        let mut deltas = String::new();
        let stream = ChatStreamAdapter::<crate::StreamHandle>::new(stream).into_stream();
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
        let req = ProxyRequest::builder(Model::OpenAIGpt4oMini)
            .provider(Provider::OpenAI)
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
                    Model::OpenAIGpt4oMini,
                    vec![ProxyMessage {
                        role: "user".into(),
                        content: "hi".into(),
                    }],
                )
                .unwrap(),
                BlockingProxyOptions::default(),
            )
            .unwrap();
        assert_eq!(resp.provider, Provider::OpenAI);
    }
}
