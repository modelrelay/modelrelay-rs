use std::path::PathBuf;

use modelrelay::{
    validate_workflow_spec_v0, workflow_v0, ExecutionV0, ResponseBuilder, WorkflowSpecV0,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read_fixture(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read fixture")
}

#[test]
fn builds_parallel_agents_fixture() {
    let fixture_json = read_fixture("platform/workflow/testdata/workflow_v0_parallel_agents.json");
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
fn conformance_invalid_fixtures_match_codes() {
    let cases = [
        (
            "platform/workflow/testdata/workflow_v0_invalid_duplicate_node_id.json",
            "platform/workflow/testdata/workflow_v0_invalid_duplicate_node_id.issues.json",
        ),
        (
            "platform/workflow/testdata/workflow_v0_invalid_edge_unknown_node.json",
            "platform/workflow/testdata/workflow_v0_invalid_edge_unknown_node.issues.json",
        ),
        (
            "platform/workflow/testdata/workflow_v0_invalid_output_unknown_node.json",
            "platform/workflow/testdata/workflow_v0_invalid_output_unknown_node.issues.json",
        ),
    ];

    for (spec_rel, issues_rel) in cases {
        let spec_json = read_fixture(spec_rel);
        let spec: WorkflowSpecV0 = serde_json::from_str(&spec_json).expect("parse spec");

        let want_codes: Vec<String> =
            serde_json::from_str(&read_fixture(issues_rel)).expect("parse issues");
        let mut want = want_codes;
        want.sort();

        let mut got: Vec<String> = validate_workflow_spec_v0(&spec)
            .into_iter()
            .map(|i| i.code.as_str().to_string())
            .collect();
        got.sort();

        assert_eq!(got, want, "fixture {spec_rel} codes mismatch");
    }
}
