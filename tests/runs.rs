use futures_util::StreamExt;
use modelrelay::{
    ApiKey, ArtifactKey, Client, Config, EdgeV0, EnvelopeVersion, NodeId, NodeTypeV0, NodeV0,
    OutputRefV0, RunEventPayload, RunId, RunStatusV0, WorkflowKind, WorkflowSpecV0,
};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client_for_server(server: &MockServer) -> Client {
    Client::new(Config {
        api_key: Some(ApiKey::parse("mr_sk_test_key").unwrap()),
        base_url: Some(server.uri()),
        retry: Some(modelrelay::RetryConfig {
            max_attempts: 1,
            ..Default::default()
        }),
        ..Default::default()
    })
    .expect("client creation should succeed")
}

#[tokio::test]
async fn runs_create_get_and_stream_events() {
    let server = MockServer::start().await;

    let run_id: RunId = "11111111-1111-1111-1111-111111111111".parse().unwrap();
    let plan_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let node_a: NodeId = "a".parse().unwrap();
    let node_b: NodeId = "b".parse().unwrap();

    let spec = WorkflowSpecV0 {
        kind: WorkflowKind::WorkflowV0,
        name: None,
        execution: None,
        nodes: vec![
            NodeV0 {
                id: node_a.clone(),
                node_type: NodeTypeV0::JoinAll,
                input: None,
            },
            NodeV0 {
                id: node_b.clone(),
                node_type: NodeTypeV0::JoinAll,
                input: None,
            },
        ],
        edges: Some(vec![EdgeV0 {
            from: node_a,
            to: node_b.clone(),
        }]),
        outputs: vec![OutputRefV0 {
            name: "result".to_string(),
            from: node_b,
            pointer: None,
        }],
    };

    Mock::given(method("POST"))
        .and(path("/runs"))
        .and(body_json(
            json!({ "spec": serde_json::to_value(&spec).unwrap() }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "run_id": run_id.to_string(),
            "status": "running",
            "plan_hash": plan_hash
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/runs/{}", run_id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "run_id": run_id.to_string(),
            "status": "running",
            "plan_hash": plan_hash,
            "cost_summary": { "total_usd_cents": 0, "line_items": [] },
            "nodes": [],
            "outputs": {}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let ndjson = format!(
        "{}\n{}\n",
        json!({
            "envelope_version": "v0",
            "run_id": run_id.to_string(),
            "seq": 1,
            "ts": "2025-12-14T00:00:00Z",
            "type": "run_started",
            "plan_hash": plan_hash
        }),
        json!({
            "envelope_version": "v0",
            "run_id": run_id.to_string(),
            "seq": 2,
            "ts": "2025-12-14T00:00:00Z",
            "type": "run_completed",
            "plan_hash": plan_hash,
            "outputs_artifact_key": "run_outputs.v0",
            "outputs_info": {
                "bytes": 0,
                "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "included": false
            }
        })
    );

    Mock::given(method("GET"))
        .and(path(format!("/runs/{}/events", run_id)))
        .respond_with(ResponseTemplate::new(200).set_body_raw(ndjson, "application/x-ndjson"))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for_server(&server);

    let created = client
        .runs()
        .create(spec)
        .await
        .expect("create should succeed");
    assert_eq!(created.run_id, run_id);
    assert_eq!(created.status, RunStatusV0::Running);

    let snap = client
        .runs()
        .get(run_id.clone())
        .await
        .expect("get should succeed");
    assert_eq!(snap.run_id, run_id);
    assert_eq!(snap.plan_hash.to_string(), plan_hash);

    let mut stream = client
        .runs()
        .stream_events(run_id.clone(), None, None)
        .await
        .expect("stream should succeed");

    let mut events = vec![];
    while let Some(item) = stream.next().await {
        events.push(item.expect("event should parse"));
    }
    assert_eq!(events.len(), 2);

    // Test envelope fields directly (no pattern matching needed)
    assert_eq!(events[0].envelope.envelope_version, EnvelopeVersion::V0);
    assert_eq!(events[0].envelope.seq, 1);

    // Test payload with pattern matching
    match &events[0].payload {
        RunEventPayload::RunStarted {
            plan_hash: got_hash,
        } => {
            assert_eq!(got_hash.to_string(), plan_hash);
        }
        other => panic!("expected RunStarted, got {other:?}"),
    }

    // Second event
    assert_eq!(events[1].envelope.envelope_version, EnvelopeVersion::V0);
    assert_eq!(events[1].envelope.seq, 2);

    match &events[1].payload {
        RunEventPayload::RunCompleted {
            plan_hash: got_hash,
            outputs_artifact_key,
            outputs_info,
        } => {
            assert_eq!(got_hash.to_string(), plan_hash);
            assert_eq!(outputs_artifact_key.as_str(), ArtifactKey::RUN_OUTPUTS_V0);
            assert_eq!(outputs_info.included, false);
        }
        other => panic!("expected RunCompleted, got {other:?}"),
    }
}
