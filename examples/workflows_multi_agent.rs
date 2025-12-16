use futures_util::StreamExt;
use modelrelay::{
    workflow_v0, ApiKey, Client, Config, ExecutionV0, LlmResponsesBindingV0, NodeId,
    ResponseBuilder, RunEventPayload, WorkflowSpecV0,
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

fn multi_agent_spec(
    model_a: &str,
    model_b: &str,
    model_c: &str,
    model_agg: &str,
    run_timeout_ms: i64,
) -> ExampleResult<WorkflowSpecV0> {
    let exec = ExecutionV0 {
        max_parallelism: Some(3),
        node_timeout_ms: Some(20_000),
        run_timeout_ms: Some(if run_timeout_ms == 0 {
            30_000
        } else {
            run_timeout_ms
        }),
    };

    let agent_a: NodeId = "agent_a".parse().unwrap();
    let agent_b: NodeId = "agent_b".parse().unwrap();
    let agent_c: NodeId = "agent_c".parse().unwrap();
    let join: NodeId = "join".parse().unwrap();
    let aggregate: NodeId = "aggregate".parse().unwrap();

    let spec = workflow_v0()
        .name("multi_agent_v0_example")
        .execution(exec)
        .llm_responses(
            agent_a.clone(),
            ResponseBuilder::new()
                .model(model_a)
                .max_output_tokens(64)
                .system("You are Agent A.")
                .user("Write 3 ideas for a landing page."),
            Some(false),
        )?
        .llm_responses(
            agent_b.clone(),
            ResponseBuilder::new()
                .model(model_b)
                .max_output_tokens(64)
                .system("You are Agent B.")
                .user("Write 3 objections a user might have."),
            None,
        )?
        .llm_responses(
            agent_c.clone(),
            ResponseBuilder::new()
                .model(model_c)
                .max_output_tokens(64)
                .system("You are Agent C.")
                .user("Write 3 alternative headlines."),
            None,
        )?
        .join_all(join.clone())
        .llm_responses_with_bindings(
            aggregate.clone(),
            ResponseBuilder::new()
                .model(model_agg)
                .max_output_tokens(256)
                .system("Synthesize the best answer from the following agent outputs (JSON).")
                .user(""), // overwritten by bindings
            None,
            Some(vec![LlmResponsesBindingV0::json_string(
                join.clone(),
                None,
                "/input/1/content/0/text",
            )]),
        )?
        .edge(agent_a, join.clone())
        .edge(agent_b, join.clone())
        .edge(agent_c, join.clone())
        .edge(join, aggregate.clone())
        .output("result", aggregate, None)
        .build()?;

    Ok(spec)
}

async fn run_once(client: &Client, label: &str, spec: WorkflowSpecV0) -> ExampleResult<()> {
    println!(
        "[{label}] compiled workflow.v0: {}",
        serde_json::to_string_pretty(&spec)?
    );

    let created = client.runs().create(spec).await?;
    let run_id = created.run_id;
    println!("[{label}] run_id={}", run_id);

    let mut stream = client
        .runs()
        .stream_events(run_id.clone(), None, None)
        .await?;

    while let Some(item) = stream.next().await {
        let event = item?;
        match &event.payload {
            RunEventPayload::RunCompleted { .. } => {
                let snap = client.runs().get(run_id.clone()).await?;
                println!(
                    "[{label}] outputs: {}",
                    serde_json::to_string_pretty(&snap.outputs)?
                );
            }
            RunEventPayload::RunFailed { error, .. } => {
                println!("[{label}] run_failed: {}", error.message);
            }
            RunEventPayload::RunCanceled { error, .. } => {
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

    run_once(
        &client,
        "success",
        multi_agent_spec(&model_ok, &model_ok, &model_ok, &model_ok, 0)?,
    )
    .await?;

    run_once(
        &client,
        "partial_failure",
        multi_agent_spec(&model_ok, &model_bad, &model_ok, &model_ok, 0)?,
    )
    .await?;

    run_once(
        &client,
        "cancellation",
        multi_agent_spec(&model_ok, &model_ok, &model_ok, &model_ok, 1)?,
    )
    .await?;

    Ok(())
}
