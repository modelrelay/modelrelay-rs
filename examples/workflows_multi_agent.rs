use futures_util::StreamExt;
use modelrelay::{
    ApiKey, Client, Config, EdgeV0, ExecutionV0, NodeId, NodeTypeV0, NodeV0, OutputRefV0,
    RunEventV0, WorkflowKind, WorkflowSpecV0,
};
use serde::Deserialize;
use serde_json::json;
use std::{error::Error, fmt};

#[derive(Debug, Deserialize)]
struct DevLoginResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct MeResponse {
    user: MeUser,
}

#[derive(Debug, Deserialize)]
struct MeUser {
    project_id: String,
}

#[derive(Debug, Deserialize)]
struct APIKeyCreateResponse {
    api_key: APIKeyPayload,
}

#[derive(Debug, Deserialize)]
struct APIKeyPayload {
    secret_key: Option<String>,
}

#[derive(Debug)]
struct ExampleError(String);

impl fmt::Display for ExampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for ExampleError {}

type ExampleResult<T> = std::result::Result<T, Box<dyn Error>>;

fn arg_value(flag: &str) -> Option<String> {
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        if arg == flag {
            return it.next();
        }
    }
    None
}

async fn bootstrap_secret_key(api_base_url: &str) -> ExampleResult<String> {
    let http = reqwest::Client::new();

    let login: DevLoginResponse = http
        .post(format!("{api_base_url}/auth/dev-login"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if login.access_token.trim().is_empty() {
        return Err(Box::new(ExampleError(
            "dev-login returned empty access_token".to_string(),
        )));
    }

    let me: MeResponse = http
        .get(format!("{api_base_url}/auth/me"))
        .header("Authorization", format!("Bearer {}", login.access_token))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if me.user.project_id.trim().is_empty() {
        return Err(Box::new(ExampleError(
            "auth/me returned empty user.project_id".to_string(),
        )));
    }

    let created: APIKeyCreateResponse = http
        .post(format!("{api_base_url}/api-keys"))
        .header("Authorization", format!("Bearer {}", login.access_token))
        .json(&json!({
            "label": "Workflows example (dev)",
            "project_id": me.user.project_id,
            "kind": "secret",
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let secret_key = created.api_key.secret_key.ok_or_else(|| {
        Box::new(ExampleError(
            "api-keys create response missing api_key.secret_key".to_string(),
        )) as Box<dyn Error>
    })?;

    Ok(secret_key)
}

fn multi_agent_spec(model: &str) -> WorkflowSpecV0 {
    WorkflowSpecV0 {
        kind: WorkflowKind::WorkflowV0,
        name: Some("multi_agent_v0_example".to_string()),
        execution: Some(ExecutionV0 {
            max_parallelism: Some(3),
            node_timeout_ms: Some(20_000),
            run_timeout_ms: Some(30_000),
        }),
        nodes: vec![
            NodeV0 {
                id: NodeId::from("agent_a"),
                node_type: NodeTypeV0::LlmResponses,
                input: Some(json!({
                    "request": {
                        "model": model,
                        "input": [
                            { "type": "message", "role": "system", "content": [{ "type": "text", "text": "You are Agent A." }] },
                            { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Write 3 ideas for a landing page." }] }
                        ]
                    }
                })),
            },
            NodeV0 {
                id: NodeId::from("agent_b"),
                node_type: NodeTypeV0::LlmResponses,
                input: Some(json!({
                    "request": {
                        "model": model,
                        "input": [
                            { "type": "message", "role": "system", "content": [{ "type": "text", "text": "You are Agent B." }] },
                            { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Write 3 objections a user might have." }] }
                        ]
                    }
                })),
            },
            NodeV0 {
                id: NodeId::from("agent_c"),
                node_type: NodeTypeV0::LlmResponses,
                input: Some(json!({
                    "request": {
                        "model": model,
                        "input": [
                            { "type": "message", "role": "system", "content": [{ "type": "text", "text": "You are Agent C." }] },
                            { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Write 3 alternative headlines." }] }
                        ]
                    }
                })),
            },
            NodeV0 {
                id: NodeId::from("join"),
                node_type: NodeTypeV0::JoinAll,
                input: None,
            },
            NodeV0 {
                id: NodeId::from("aggregate"),
                node_type: NodeTypeV0::TransformJson,
                input: Some(json!({
                    "object": {
                        "agent_a": { "from": "join", "pointer": "/agent_a" },
                        "agent_b": { "from": "join", "pointer": "/agent_b" },
                        "agent_c": { "from": "join", "pointer": "/agent_c" }
                    }
                })),
            },
        ],
        edges: Some(vec![
            EdgeV0 {
                from: NodeId::from("agent_a"),
                to: NodeId::from("join"),
            },
            EdgeV0 {
                from: NodeId::from("agent_b"),
                to: NodeId::from("join"),
            },
            EdgeV0 {
                from: NodeId::from("agent_c"),
                to: NodeId::from("join"),
            },
            EdgeV0 {
                from: NodeId::from("join"),
                to: NodeId::from("aggregate"),
            },
        ]),
        outputs: vec![OutputRefV0 {
            name: "result".to_string(),
            from: NodeId::from("aggregate"),
            pointer: None,
        }],
    }
}

async fn run_once(client: &Client, label: &str, spec: WorkflowSpecV0) -> ExampleResult<()> {
    let created = client.runs().create(spec).await?;
    println!("[{label}] run_id={}", created.run_id);

    let mut stream = client
        .runs()
        .stream_events(created.run_id, None, None)
        .await?;

    while let Some(item) = stream.next().await {
        match item? {
            RunEventV0::RunCompleted { outputs, .. } => {
                println!(
                    "[{label}] outputs: {}",
                    serde_json::to_string_pretty(&outputs)?
                );
            }
            RunEventV0::RunFailed { error, .. } => {
                println!("[{label}] run_failed: {}", error.message);
            }
            RunEventV0::RunCanceled { error, .. } => {
                println!("[{label}] run_canceled: {}", error.message);
            }
            _ => {}
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExampleResult<()> {
    let api_base_url = arg_value("--base-url")
        .or_else(|| std::env::var("MODELRELAY_API_BASE_URL").ok())
        .unwrap_or_else(|| "http://localhost:8080/api/v1".to_string());

    let model_ok = std::env::var("MODELRELAY_MODEL_OK")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
    let model_bad = std::env::var("MODELRELAY_MODEL_BAD")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "does-not-exist".to_string());

    let secret = bootstrap_secret_key(&api_base_url).await?;
    let client = Client::new(Config {
        api_key: Some(ApiKey::parse(&secret)?),
        base_url: Some(api_base_url.clone()),
        ..Default::default()
    })?;

    run_once(&client, "success", multi_agent_spec(&model_ok)).await?;

    let mut fail_spec = multi_agent_spec(&model_ok);
    if let Some(nodes) = fail_spec
        .nodes
        .iter_mut()
        .find(|n| n.id.as_str() == "agent_b")
    {
        nodes.input = Some(json!({
            "request": {
                "model": model_bad,
                "input": [
                    { "type": "message", "role": "system", "content": [{ "type": "text", "text": "You are Agent B." }] },
                    { "type": "message", "role": "user", "content": [{ "type": "text", "text": "Write 3 objections a user might have." }] }
                ]
            }
        }));
    }
    run_once(&client, "partial_failure", fail_spec).await?;

    let mut cancel_spec = multi_agent_spec(&model_ok);
    if let Some(exec) = cancel_spec.execution.as_mut() {
        exec.run_timeout_ms = Some(1);
    }
    run_once(&client, "cancellation", cancel_spec).await?;

    Ok(())
}
