#![cfg(all(feature = "blocking", feature = "mock"))]

use modelrelay::{
    MockClient, MockConfig, Model, ProxyMessage, ProxyOptions, ProxyRequest, fixtures,
};

#[test]
fn blocking_proxy_uses_mock_queue() {
    let mut resp = fixtures::simple_proxy_response();
    resp.request_id = None;

    let client = MockClient::new(MockConfig::default().with_proxy_response(resp.clone()));

    let result = client
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
            ProxyOptions::default().with_request_id("req_blocking_mock"),
        )
        .unwrap();

    assert_eq!(result.content.join(""), resp.content.join(""));
    assert_eq!(result.request_id.as_deref(), Some("req_blocking_mock"));
}

#[cfg(feature = "streaming")]
#[test]
fn blocking_stream_adapter_yields_deltas() {
    use modelrelay::{BlockingProxyHandle, ChatStreamAdapter, StopReason};

    let mut override_events = fixtures::simple_stream_events();
    for evt in override_events.iter_mut() {
        evt.request_id = None;
    }
    let client = MockClient::new(
        MockConfig::default()
            .with_stream_events(override_events)
            .with_stream_events(fixtures::simple_stream_events()),
    );

    let request = ProxyRequest::new(
        Model::OpenAIGpt4oMini,
        vec![ProxyMessage {
            role: "user".into(),
            content: "stream it".into(),
        }],
    )
    .unwrap();

    let stream = client
        .blocking_llm()
        .proxy_stream(
            request.clone(),
            ProxyOptions::default().with_request_id("req_blocking_stream_override"),
        )
        .unwrap();

    let mut adapter = ChatStreamAdapter::<BlockingProxyHandle>::new(stream);
    let mut deltas = String::new();
    while let Some(delta) = adapter.next_delta().unwrap() {
        deltas.push_str(&delta);
    }

    assert_eq!(deltas, "hello");
    assert_eq!(
        adapter.final_request_id(),
        Some("req_blocking_stream_override")
    );
    assert_eq!(adapter.final_stop_reason(), Some(&StopReason::Completed));
    assert!(adapter.final_usage().is_some());

    let stream2 = client
        .blocking_llm()
        .proxy_stream(request, ProxyOptions::default())
        .unwrap();
    let collected = stream2.collect().unwrap();
    assert_eq!(collected.content.join(""), "hello");
    assert_eq!(collected.request_id.as_deref(), Some("req_stream_mock"));
}
