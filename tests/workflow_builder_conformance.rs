use std::path::PathBuf;

use modelrelay::{
    validate_workflow_spec_v0, workflow_v0, ExecutionV0, LlmResponsesBindingV0, ResponseBuilder,
    WorkflowSpecV0,
};
use serde::Deserialize;

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
        .build_result()
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
        .build_result()
        .unwrap();

    let got_value = serde_json::to_value(&spec).expect("serialize spec");
    assert_eq!(got_value, fixture_value);
}

#[test]
fn conformance_invalid_fixtures_match_codes() {
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
        let want = fixture_codes(raw_issues);

        let mut got: Vec<String> = validate_workflow_spec_v0(&spec)
            .into_iter()
            .map(|i| i.code.as_str().to_string())
            .collect();
        got.sort();

        assert_eq!(got, want, "fixture {spec_rel} codes mismatch");
    }
}

#[derive(Debug, Deserialize)]
struct WorkflowFixtureIssue {
    code: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowFixtureValidationError {
    issues: Vec<WorkflowFixtureIssue>,
}

fn fixture_codes(raw: String) -> Vec<String> {
    // Legacy fixtures were `[]string`. Prefer the new structured `ValidationError{issues[]}`.
    if let Ok(mut codes) = serde_json::from_str::<Vec<String>>(&raw) {
        codes.sort();
        return codes;
    }

    let verr: WorkflowFixtureValidationError =
        serde_json::from_str(&raw).expect("parse validation error fixture");

    let mut out: Vec<String> = verr
        .issues
        .into_iter()
        .filter_map(|iss| map_workflow_issue_to_sdk_code(iss))
        .collect();
    out.sort();
    out
}

fn map_workflow_issue_to_sdk_code(iss: WorkflowFixtureIssue) -> Option<String> {
    match iss.code.as_str() {
        "INVALID_KIND" => Some("invalid_kind".to_string()),
        "MISSING_NODES" => Some("missing_nodes".to_string()),
        "MISSING_OUTPUTS" => Some("missing_outputs".to_string()),
        "DUPLICATE_NODE_ID" => Some("duplicate_node_id".to_string()),
        "DUPLICATE_OUTPUT_NAME" => Some("duplicate_output_name".to_string()),
        "UNKNOWN_EDGE_ENDPOINT" => {
            if iss.path.ends_with(".from") {
                Some("edge_from_unknown_node".to_string())
            } else if iss.path.ends_with(".to") {
                Some("edge_to_unknown_node".to_string())
            } else {
                None
            }
        }
        "UNKNOWN_OUTPUT_NODE" => Some("output_from_unknown_node".to_string()),
        _ => {
            // Rust SDK preflight validation is intentionally lightweight; ignore
            // semantic issues the server/compiler can produce (e.g. join constraints).
            None
        }
    }
}
