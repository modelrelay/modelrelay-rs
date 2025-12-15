use std::path::PathBuf;

use modelrelay::{
    workflow_v0, Client, Config, ExecutionV0, LlmResponsesBindingV0, ResponseBuilder,
    WorkflowSpecV0, WorkflowsCompileResponseV0,
};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn conformance_workflows_v0_dir() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("MODELRELAY_CONFORMANCE_DIR") {
        return Some(PathBuf::from(root).join("workflows").join("v0"));
    }

    // sdk/rust/tests -> repo root
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let internal = repo_root
        .join("platform")
        .join("workflow")
        .join("testdata")
        .join("conformance")
        .join("workflows")
        .join("v0");
    if internal.join("workflow_v0_parallel_agents.json").exists() {
        return Some(internal);
    }

    None
}

fn read_fixture(name: &str) -> Option<String> {
    let base = conformance_workflows_v0_dir()?;
    Some(std::fs::read_to_string(base.join(name)).expect("read fixture"))
}

#[test]
fn builds_parallel_agents_fixture() {
    let Some(fixture_json) = read_fixture("workflow_v0_parallel_agents.json") else {
        return;
    };
    let fixture_value: serde_json::Value =
        serde_json::from_str(&fixture_json).expect("parse fixture json");

    let exec = ExecutionV0 {
        max_parallelism: Some(3),
        node_timeout_ms: Some(60_000),
        run_timeout_ms: Some(180_000),
    };

    let spec = workflow_v0()
        .name("parallel_agents_aggregate")
        .execution(exec)
        .llm_responses(
            "agent_a",
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("You are Agent A.")
                .user("Analyze the question."),
            Some(false),
        )
        .unwrap()
        .llm_responses(
            "agent_b",
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("You are Agent B.")
                .user("Find edge cases."),
            None,
        )
        .unwrap()
        .llm_responses(
            "agent_c",
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("You are Agent C.")
                .user("Propose a solution."),
            None,
        )
        .unwrap()
        .join_all("join")
        .llm_responses(
            "aggregate",
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(256)
                .system("Synthesize the best answer."),
            None,
        )
        .unwrap()
        .edge("agent_a", "join")
        .edge("agent_b", "join")
        .edge("agent_c", "join")
        .edge("join", "aggregate")
        .output("final", "aggregate", None)
        .build()
        .unwrap();

    let got_value = serde_json::to_value(&spec).expect("serialize spec");
    assert_eq!(got_value, fixture_value);
}

#[test]
fn builds_bindings_fixture() {
    let Some(fixture_json) = read_fixture("workflow_v0_bindings_join_into_aggregate.json") else {
        return;
    };
    let fixture_value: serde_json::Value =
        serde_json::from_str(&fixture_json).expect("parse fixture json");

    let spec = workflow_v0()
        .name("bindings_join_into_aggregate")
        .llm_responses(
            "agent_a",
            ResponseBuilder::new().model("echo-1").user("hello a"),
            None,
        )
        .unwrap()
        .llm_responses(
            "agent_b",
            ResponseBuilder::new().model("echo-1").user("hello b"),
            None,
        )
        .unwrap()
        .join_all("join")
        .llm_responses_with_bindings(
            "aggregate",
            ResponseBuilder::new().model("echo-1").user(""),
            None,
            Some(vec![LlmResponsesBindingV0::json_string(
                "join",
                None,
                "/input/0/content/0/text",
            )]),
        )
        .unwrap()
        .edge("agent_a", "join")
        .edge("agent_b", "join")
        .edge("join", "aggregate")
        .output(
            "final",
            "aggregate",
            Some("/output/0/content/0/text".to_string()),
        )
        .build()
        .unwrap();

    let got_value = serde_json::to_value(&spec).expect("serialize spec");
    assert_eq!(got_value, fixture_value);
}

#[tokio::test]
async fn workflows_compile_conformance_parallel_agents_fixture() {
    let Some(spec_json) = read_fixture("workflow_v0_parallel_agents.json") else {
        return;
    };
    let spec: WorkflowSpecV0 = serde_json::from_str(&spec_json).expect("parse spec");

    let Some(plan_json) = read_fixture("workflow_v0_parallel_agents.plan.json") else {
        return;
    };
    let plan_value: serde_json::Value = serde_json::from_str(&plan_json).expect("parse plan json");
    let plan_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows/compile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "plan_json": plan_value.clone(), "plan_hash": plan_hash }),
        ))
        .mount(&server)
        .await;

    let client = Client::new(Config {
        base_url: Some(format!("{}/api/v1", server.uri())),
        api_key: Some(modelrelay::ApiKey::parse("mr_sk_test").unwrap()),
        ..Default::default()
    })
    .unwrap();

    let got: WorkflowsCompileResponseV0 = client.workflows().compile_v0(spec).await.unwrap();
    assert_eq!(got.plan_hash.to_string(), plan_hash);
    assert_eq!(got.plan_json, plan_value);
}

#[tokio::test]
async fn workflows_compile_conformance_invalid_fixtures_surface_issues() {
    if conformance_workflows_v0_dir().is_none() {
        return;
    }

    let cases = [
        (
            "workflow_v0_invalid_duplicate_node_id.json",
            "workflow_v0_invalid_duplicate_node_id.issues.json",
        ),
        (
            "workflow_v0_invalid_edge_unknown_node.json",
            "workflow_v0_invalid_edge_unknown_node.issues.json",
        ),
        (
            "workflow_v0_invalid_output_unknown_node.json",
            "workflow_v0_invalid_output_unknown_node.issues.json",
        ),
    ];

    for (spec_rel, issues_rel) in cases {
        let Some(spec_json) = read_fixture(spec_rel) else {
            return;
        };
        let spec: WorkflowSpecV0 = serde_json::from_str(&spec_json).expect("parse spec");

        let Some(raw_issues) = read_fixture(issues_rel) else {
            return;
        };
        let want: modelrelay::WorkflowValidationError =
            serde_json::from_str(&raw_issues).expect("parse validation error fixture");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/workflows/compile"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(raw_issues.clone())
                    .insert_header("Content-Type", "application/json"),
            )
            .mount(&server)
            .await;

        let client = Client::new(Config {
            base_url: Some(format!("{}/api/v1", server.uri())),
            api_key: Some(modelrelay::ApiKey::parse("mr_sk_test").unwrap()),
            ..Default::default()
        })
        .unwrap();

        let err = client.workflows().compile_v0(spec).await.unwrap_err();
        match err {
            modelrelay::Error::WorkflowValidation(got) => {
                assert_eq!(
                    got.issues, want.issues,
                    "fixture {spec_rel} issues mismatch"
                );
            }
            other => panic!("expected workflow validation error, got {other:?}"),
        }
    }
}
