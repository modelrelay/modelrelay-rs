use std::path::PathBuf;

use modelrelay::{
    new_workflow, workflow_v0, workflow_v1, Client, ConditionOpV1, ConditionSourceV1, ConditionV1,
    Config, ExecutionV0, ExecutionV1, LlmResponsesBindingEncodingV0, LlmResponsesBindingEncodingV1,
    LlmResponsesBindingV0, LlmResponsesBindingV1, LlmResponsesNodeOptionsV1, MapFanoutInputV1,
    MapFanoutItemBindingV1, MapFanoutItemsV1, MapFanoutSubNodeV1, NodeId, ResponseBuilder,
    WorkflowSpecV0, WorkflowSpecV1, WorkflowsCompileResultV0, WorkflowsCompileResultV1,
};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[derive(serde::Deserialize)]
struct PlanHashFixture {
    plan_hash: String,
}

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

fn conformance_workflows_v1_dir() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("MODELRELAY_CONFORMANCE_DIR") {
        return Some(PathBuf::from(root).join("workflows").join("v1"));
    }

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let internal = repo_root
        .join("platform")
        .join("workflow")
        .join("testdata")
        .join("conformance")
        .join("workflows")
        .join("v1");
    if internal.join("workflow_v1_router.json").exists() {
        return Some(internal);
    }

    None
}

fn read_fixture_v1(name: &str) -> Option<String> {
    let base = conformance_workflows_v1_dir()?;
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

    let agent_a: NodeId = "agent_a".parse().unwrap();
    let agent_b: NodeId = "agent_b".parse().unwrap();
    let agent_c: NodeId = "agent_c".parse().unwrap();
    let join: NodeId = "join".parse().unwrap();
    let aggregate: NodeId = "aggregate".parse().unwrap();

    let spec = workflow_v0()
        .name("parallel_agents_aggregate")
        .execution(exec)
        .llm_responses(
            agent_a.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("You are Agent A.")
                .user("Analyze the question."),
            Some(false),
        )
        .unwrap()
        .llm_responses(
            agent_b.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("You are Agent B.")
                .user("Find edge cases."),
            None,
        )
        .unwrap()
        .llm_responses(
            agent_c.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("You are Agent C.")
                .user("Propose a solution."),
            None,
        )
        .unwrap()
        .join_all(join.clone())
        .llm_responses(
            aggregate.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(256)
                .system("Synthesize the best answer."),
            None,
        )
        .unwrap()
        .edge(agent_a, join.clone())
        .edge(agent_b, join.clone())
        .edge(agent_c, join.clone())
        .edge(join, aggregate.clone())
        .output("final", aggregate, None)
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

    let agent_a: NodeId = "agent_a".parse().unwrap();
    let agent_b: NodeId = "agent_b".parse().unwrap();
    let join: NodeId = "join".parse().unwrap();
    let aggregate: NodeId = "aggregate".parse().unwrap();

    let spec = workflow_v0()
        .name("bindings_join_into_aggregate")
        .llm_responses(
            agent_a.clone(),
            ResponseBuilder::new().model("echo-1").user("hello a"),
            None,
        )
        .unwrap()
        .llm_responses(
            agent_b.clone(),
            ResponseBuilder::new().model("echo-1").user("hello b"),
            None,
        )
        .unwrap()
        .join_all(join.clone())
        .llm_responses_with_bindings(
            aggregate.clone(),
            ResponseBuilder::new().model("echo-1").user(""),
            None,
            Some(vec![LlmResponsesBindingV0::json_string(
                join.clone(),
                None,
                "/input/0/content/0/text",
            )]),
        )
        .unwrap()
        .edge(agent_a, join.clone())
        .edge(agent_b, join.clone())
        .edge(join, aggregate.clone())
        .output(
            "final",
            aggregate,
            Some("/output/0/content/0/text".to_string()),
        )
        .build()
        .unwrap();

    let got_value = serde_json::to_value(&spec).expect("serialize spec");
    assert_eq!(got_value, fixture_value);
}

/// Test ergonomic builder with auto-edge inference from bindings.
#[test]
fn ergonomic_builder_with_bindings() {
    let Some(fixture_json) = read_fixture("workflow_v0_bindings_join_into_aggregate.json") else {
        return;
    };
    let fixture_value: serde_json::Value =
        serde_json::from_str(&fixture_json).expect("parse fixture json");

    let agent_a: NodeId = "agent_a".parse().unwrap();
    let agent_b: NodeId = "agent_b".parse().unwrap();
    let join: NodeId = "join".parse().unwrap();
    let aggregate: NodeId = "aggregate".parse().unwrap();

    // Use the new ergonomic builder with auto-edge inference
    let spec = new_workflow("bindings_join_into_aggregate")
        .add_llm_node(
            agent_a.clone(),
            ResponseBuilder::new().model("echo-1").user("hello a"),
        )
        .unwrap()
        .add_llm_node(
            agent_b.clone(),
            ResponseBuilder::new().model("echo-1").user("hello b"),
        )
        .unwrap()
        .add_join_all_node(join.clone())
        .add_llm_node(
            aggregate.clone(),
            ResponseBuilder::new().model("echo-1").user(""),
        )
        .unwrap()
        // bind_from_to auto-infers edge from "join" to current node
        .bind_from_to(
            join.clone(),
            None,
            "/input/0/content/0/text",
            Some(LlmResponsesBindingEncodingV0::JsonString),
        )
        .edge(agent_a, join.clone())
        .edge(agent_b, join.clone())
        .edge(join, aggregate.clone())
        .output(
            "final",
            aggregate,
            Some("/output/0/content/0/text".to_string()),
        )
        .build()
        .unwrap();

    let got_value = serde_json::to_value(&spec).expect("serialize spec");
    assert_eq!(got_value, fixture_value);
}

/// Test ergonomic builder produces same output as classic builder.
#[test]
fn ergonomic_builder_matches_classic() {
    let exec = ExecutionV0 {
        max_parallelism: Some(3),
        node_timeout_ms: Some(60_000),
        run_timeout_ms: Some(180_000),
    };

    let agent_a: NodeId = "agent_a".parse().unwrap();
    let agent_b: NodeId = "agent_b".parse().unwrap();
    let join: NodeId = "join".parse().unwrap();
    let aggregate: NodeId = "aggregate".parse().unwrap();

    // Classic builder
    let classic_spec = workflow_v0()
        .name("test_workflow")
        .execution(exec.clone())
        .llm_responses(
            agent_a.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("Agent A")
                .user("hello"),
            Some(true),
        )
        .unwrap()
        .llm_responses(
            agent_b.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("Agent B")
                .user("world"),
            None,
        )
        .unwrap()
        .join_all(join.clone())
        .llm_responses(
            aggregate.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(256)
                .system("Aggregator"),
            None,
        )
        .unwrap()
        .edge(agent_a.clone(), join.clone())
        .edge(agent_b.clone(), join.clone())
        .edge(join.clone(), aggregate.clone())
        .output("final", aggregate.clone(), None)
        .build()
        .unwrap();

    // Ergonomic builder
    let ergonomic_spec = new_workflow("test_workflow")
        .execution(exec)
        .add_llm_node(
            agent_a.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("Agent A")
                .user("hello"),
        )
        .unwrap()
        .stream(true)
        .add_llm_node(
            agent_b.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(64)
                .system("Agent B")
                .user("world"),
        )
        .unwrap()
        .add_join_all_node(join.clone())
        .add_llm_node(
            aggregate.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(256)
                .system("Aggregator"),
        )
        .unwrap()
        .edge(agent_a.clone(), join.clone())
        .edge(agent_b.clone(), join.clone())
        .edge(join.clone(), aggregate.clone())
        .output("final", aggregate, None)
        .build()
        .unwrap();

    // Compare serialized output
    let classic_value = serde_json::to_value(&classic_spec).expect("serialize classic");
    let ergonomic_value = serde_json::to_value(&ergonomic_spec).expect("serialize ergonomic");
    assert_eq!(classic_value, ergonomic_value);
}

#[test]
fn builds_router_fixture_v1() {
    let Some(fixture_json) = read_fixture_v1("workflow_v1_router.json") else {
        return;
    };
    let fixture_value: serde_json::Value =
        serde_json::from_str(&fixture_json).expect("parse fixture json");

    let exec = ExecutionV1 {
        max_parallelism: Some(4),
        node_timeout_ms: Some(60_000),
        run_timeout_ms: Some(180_000),
    };

    let router: NodeId = "router".parse().unwrap();
    let billing: NodeId = "billing_agent".parse().unwrap();
    let support: NodeId = "support_agent".parse().unwrap();
    let join: NodeId = "join".parse().unwrap();
    let aggregate: NodeId = "aggregate".parse().unwrap();

    let cond_billing = ConditionV1 {
        source: ConditionSourceV1::NodeOutput,
        op: ConditionOpV1::Equals,
        path: "$.route".to_string(),
        value: Some(serde_json::json!("billing")),
    };
    let cond_support = ConditionV1 {
        source: ConditionSourceV1::NodeOutput,
        op: ConditionOpV1::Equals,
        path: "$.route".to_string(),
        value: Some(serde_json::json!("support")),
    };

    let spec = workflow_v1()
        .name("router_specialists")
        .execution(exec)
        .route_switch(
            router.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(32)
                .system("Return JSON with a single 'route' field.")
                .user("Classify the request into billing or support."),
            None,
        )
        .unwrap()
        .llm_responses(
            billing.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(128)
                .system("You are a billing specialist.")
                .user("Handle the billing request."),
            None,
        )
        .unwrap()
        .llm_responses(
            support.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(128)
                .system("You are a support specialist.")
                .user("Handle the support request."),
            None,
        )
        .unwrap()
        .join_any(join.clone(), None)
        .unwrap()
        .llm_responses_with_bindings(
            aggregate.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(256)
                .system("Summarize the specialist output: {{route_output}}"),
            None,
            Some(vec![LlmResponsesBindingV1 {
                from: join.clone(),
                pointer: None,
                to: None,
                to_placeholder: Some("route_output".to_string()),
                encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
            }]),
        )
        .unwrap()
        .edge_when(router.clone(), billing.clone(), cond_billing)
        .edge_when(router.clone(), support.clone(), cond_support)
        .edge(billing, join.clone())
        .edge(support, join.clone())
        .edge(join, aggregate.clone())
        .output("final", aggregate, None)
        .build()
        .unwrap();

    let got_value = serde_json::to_value(&spec).expect("serialize spec");
    assert_eq!(got_value, fixture_value);
}

#[test]
fn builds_fanout_fixture_v1() {
    let Some(fixture_json) = read_fixture_v1("workflow_v1_fanout.json") else {
        return;
    };
    let fixture_value: serde_json::Value =
        serde_json::from_str(&fixture_json).expect("parse fixture json");

    let generator: NodeId = "question_generator".parse().unwrap();
    let fanout: NodeId = "fanout".parse().unwrap();
    let aggregate: NodeId = "aggregate".parse().unwrap();

    let subnode = MapFanoutSubNodeV1::llm_responses(
        "mapper".parse().unwrap(),
        ResponseBuilder::new()
            .model("echo-1")
            .max_output_tokens(128)
            .system("Answer the question: {{question}}"),
        LlmResponsesNodeOptionsV1::default(),
    )
    .unwrap();

    let map_input = MapFanoutInputV1 {
        items: MapFanoutItemsV1 {
            from: generator.clone(),
            path: Some("$.questions".to_string()),
        },
        item_bindings: Some(vec![MapFanoutItemBindingV1 {
            path: Some("$".to_string()),
            to: None,
            to_placeholder: Some("question".to_string()),
            encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
        }]),
        subnode,
        max_parallelism: Some(4),
    };

    let spec = workflow_v1()
        .name("fanout_questions")
        .llm_responses(
            generator.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(128)
                .system("Return JSON with a 'questions' array.")
                .user("Generate 3 subquestions."),
            None,
        )
        .unwrap()
        .map_fanout(fanout.clone(), map_input)
        .unwrap()
        .llm_responses_with_bindings(
            aggregate.clone(),
            ResponseBuilder::new()
                .model("echo-1")
                .max_output_tokens(256)
                .system("Combine the answers: "),
            None,
            Some(vec![LlmResponsesBindingV1 {
                from: fanout.clone(),
                pointer: Some("/results".to_string()),
                to: Some("/input/0/content/0/text".to_string()),
                to_placeholder: None,
                encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
            }]),
        )
        .unwrap()
        .edge(generator, fanout.clone())
        .edge(fanout, aggregate.clone())
        .output("final", aggregate, None)
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
    let plan_hash: PlanHashFixture = serde_json::from_str(
        &read_fixture("workflow_v0_parallel_agents.plan_hash.json").expect("plan hash fixture"),
    )
    .expect("parse plan hash json");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows/compile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "plan_json": plan_value.clone(), "plan_hash": plan_hash.plan_hash }),
        ))
        .mount(&server)
        .await;

    let client = Client::new(Config {
        base_url: Some(format!("{}/api/v1", server.uri())),
        api_key: Some(modelrelay::ApiKey::parse("mr_sk_test").unwrap()),
        ..Default::default()
    })
    .unwrap();

    let got = client.workflows().compile_v0(spec).await.unwrap();
    match got {
        WorkflowsCompileResultV0::Ok(out) => {
            assert_eq!(out.plan_hash.to_string(), plan_hash.plan_hash);
            assert_eq!(out.plan_json, plan_value);
        }
        other => panic!("expected ok compile result, got {other:?}"),
    }
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

        let got = client.workflows().compile_v0(spec).await.unwrap();
        match got {
            WorkflowsCompileResultV0::ValidationError(got) => {
                assert_eq!(
                    got.issues, want.issues,
                    "fixture {spec_rel} issues mismatch"
                );
            }
            other => panic!("expected workflow validation error, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn workflows_compile_conformance_router_fixture_v1() {
    let Some(spec_json) = read_fixture_v1("workflow_v1_router.json") else {
        return;
    };
    let spec: WorkflowSpecV1 = serde_json::from_str(&spec_json).expect("parse spec");

    let Some(plan_json) = read_fixture_v1("workflow_v1_router.plan.json") else {
        return;
    };
    let plan_value: serde_json::Value = serde_json::from_str(&plan_json).expect("parse plan json");
    let plan_hash: PlanHashFixture = serde_json::from_str(
        &read_fixture_v1("workflow_v1_router.plan_hash.json").expect("plan hash fixture"),
    )
    .expect("parse plan hash json");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/workflows/compile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "plan_json": plan_value.clone(), "plan_hash": plan_hash.plan_hash }),
        ))
        .mount(&server)
        .await;

    let client = Client::new(Config {
        base_url: Some(format!("{}/api/v1", server.uri())),
        api_key: Some(modelrelay::ApiKey::parse("mr_sk_test").unwrap()),
        ..Default::default()
    })
    .unwrap();

    let got = client.workflows().compile_v1(spec).await.unwrap();
    match got {
        WorkflowsCompileResultV1::Ok(out) => {
            assert_eq!(out.plan_hash.to_string(), plan_hash.plan_hash);
            assert_eq!(out.plan_json, plan_value);
        }
        other => panic!("expected ok compile result, got {other:?}"),
    }
}

#[tokio::test]
async fn workflows_compile_conformance_invalid_v1_fixtures_surface_issues() {
    if conformance_workflows_v1_dir().is_none() {
        return;
    }

    let cases = [
        (
            "workflow_v1_invalid_condition.json",
            "workflow_v1_invalid_condition.issues.json",
        ),
        (
            "workflow_v1_invalid_map_spec.json",
            "workflow_v1_invalid_map_spec.issues.json",
        ),
    ];

    for (spec_rel, issues_rel) in cases {
        let Some(spec_json) = read_fixture_v1(spec_rel) else {
            return;
        };
        let spec: WorkflowSpecV1 = serde_json::from_str(&spec_json).expect("parse spec");

        let Some(raw_issues) = read_fixture_v1(issues_rel) else {
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

        let got = client.workflows().compile_v1(spec).await.unwrap();
        match got {
            WorkflowsCompileResultV1::ValidationError(got) => {
                assert_eq!(
                    got.issues, want.issues,
                    "fixture {spec_rel} issues mismatch"
                );
            }
            other => panic!("expected workflow validation error, got {other:?}"),
        }
    }
}
