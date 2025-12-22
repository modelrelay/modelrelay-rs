# ModelRelay Rust SDK

The ModelRelay Rust SDK is a **responses-first**, **streaming-first** client for building cross-provider LLM features without committing to any single vendor API.

It’s designed to feel great in Rust:
- One fluent builder (`ResponseBuilder`) for **sync/async**, **streaming/non-streaming**, **text/structured**, and **customer-attributed** requests.
- Structured outputs powered by real Rust types (`schemars::JsonSchema` + `serde::Deserialize`) with schema generation, validation, and retry.
- A practical tool-use toolkit (registry, typed arg parsing, retry loops, streaming tool deltas) for “LLM + tools” apps.

```toml
[dependencies]
modelrelay = "0.93.0"
```

## Quick Start (Async)

```rust
use modelrelay::{Client, ResponseBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::from_api_key(std::env::var("MODELRELAY_API_KEY")?)?.build()?;

    let response = ResponseBuilder::new()
        .model("claude-sonnet-4-20250514")
        .system("Answer concisely.")
        .user("Write one line about Rust.")
        .send(&client.responses())
        .await?;

    // The response is structured: output items, tool calls, citations, usage, etc.
    // For the common case, you can extract assistant text directly:
    println!("{}", response.text());
    println!("tokens: {}", response.usage.total());
    Ok(())
}
```

## Chat-Like Text Helpers

For the most common path (**system + user → assistant text**), use the built-in convenience:

```rust
let text = client
    .responses()
    .text("claude-sonnet-4-20250514", "Answer concisely.", "Say hi.")
    .await?;
println!("{text}");
```

For customer-attributed requests where the backend selects the model:

```rust
let customer = client.for_customer("customer-123")?;
let text = customer
    .responses()
    .text("Answer concisely.", "Say hi.")
    .await?;
```

## Extracting Assistant Text

If you just need the assistant text, use:

```rust
let text = response.text();
let parts = response.text_chunks(); // each assistant text content part, in order
```

These helpers:
- include only output items with `role == assistant`
- include only `text` content parts

## Why This SDK Feels Good

### Fluent request building (value-style)

`ResponseBuilder` is a small, clonable value. You can compose “base requests” and reuse them:

```rust
use modelrelay::ResponseBuilder;

let base = ResponseBuilder::new()
    .model("gpt-4.1")
    .system("You are a careful reviewer.");

let a = base.clone().user("Summarize this changelog…");
let b = base.clone().user("Extract 3 risks…");
```

### Streaming you can actually use

If you only want text, stream just deltas:

```rust
use futures_util::StreamExt;
use modelrelay::ResponseBuilder;

let mut deltas = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Write a haiku about type systems.")
    .stream_deltas(&client.responses())
    .await?;

while let Some(delta) = deltas.next().await {
    print!("{}", delta?);
}
```

If you want full control, stream typed events (message start/delta/stop, tool deltas, ping/custom):

```rust
use futures_util::StreamExt;
use modelrelay::{ResponseBuilder, StreamEventKind};

let mut stream = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Think step by step, but only output the final answer.")
    .stream(&client.responses())
    .await?;

while let Some(evt) = stream.next().await {
    let evt = evt?;
    if evt.kind == StreamEventKind::MessageDelta {
        if let Some(text) = evt.text_delta {
            print!("{}", text);
        }
    }
}
```

## Workflow Runs (workflow.v0)

```rust
use futures_util::StreamExt;
use modelrelay::{
    workflow_v0, Client, ExecutionV0, LlmResponsesBindingV0, ResponseBuilder, RunEventTypeV0,
};

let client = Client::from_api_key(std::env::var("MODELRELAY_API_KEY")?)?.build()?;

let exec = ExecutionV0 {
    max_parallelism: Some(3),
    node_timeout_ms: Some(20_000),
    run_timeout_ms: Some(30_000),
};

let spec = workflow_v0()
    .name("multi_agent_v0_example")
    .execution(exec)
    .llm_responses(
        "agent_a",
        ResponseBuilder::new()
            .model("claude-sonnet-4-20250514")
            .max_output_tokens(64)
            .system("You are Agent A.")
            .user("Analyze the question."),
        Some(false),
    )?
    .llm_responses(
        "agent_b",
        ResponseBuilder::new()
            .model("claude-sonnet-4-20250514")
            .max_output_tokens(64)
            .system("You are Agent B.")
            .user("Find edge cases."),
        None,
    )?
    .llm_responses(
        "agent_c",
        ResponseBuilder::new()
            .model("claude-sonnet-4-20250514")
            .max_output_tokens(64)
            .system("You are Agent C.")
            .user("Propose a solution."),
        None,
    )?
    .join_all("join")
    .llm_responses_with_bindings(
        "aggregate",
        ResponseBuilder::new()
            .model("claude-sonnet-4-20250514")
            .max_output_tokens(256)
            .system("Synthesize the best answer.")
            .user(""), // overwritten by bindings
        None,
        Some(vec![LlmResponsesBindingV0::json_string(
            "join",
            None,
            "/input/1/content/0/text",
        )]),
    )?
    .edge("agent_a", "join")
    .edge("agent_b", "join")
    .edge("agent_c", "join")
    .edge("join", "aggregate")
    .output("final", "aggregate", None)
    .build()?;

let created = client.runs().create(spec).await?;
let mut stream = client.runs().stream_events(created.run_id, None, None).await?;

while let Some(item) = stream.next().await {
    let ev = item?;
    if ev.kind() == RunEventTypeV0::RunCompleted {
        let status = client.runs().get(created.run_id).await?;
        println!("outputs: {:?}", status.outputs);
    }
}
```

See the full example in `sdk/rust/examples/workflows_multi_agent.rs`.

### Structured outputs from Rust types (with retry)

Structured outputs are the “Rust-native” path: you describe a type, and you get a typed value back.

```rust
use modelrelay::{Client, ResponseBuilder};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct Person {
    name: String,
    age: u32,
    email: Option<String>,
}

let client = Client::from_api_key(std::env::var("MODELRELAY_API_KEY")?)?.build()?;

let result = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Extract: John Doe is 30 years old, john@example.com")
    .structured::<Person>()
    .max_retries(2)
    .send(&client.responses())
    .await?;

println!("{:?}", result.value);
```

And you can stream typed JSON with field-level completion for progressive UIs:

```rust
use futures_util::StreamExt;
use schemars::JsonSchema;
use serde::Deserialize;
use modelrelay::ResponseBuilder;

#[derive(Debug, Deserialize, JsonSchema)]
struct Article {
    title: String,
    summary: String,
    body: String,
}

let mut stream = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Write an article about Rust's ownership model.")
    .structured::<Article>()
    .stream(&client.responses())
    .await?;

while let Some(evt) = stream.next().await {
    let evt = evt?;
    for field in &evt.complete_fields {
        if field == "title" {
            println!("Title: {}", evt.payload.title);
        }
    }
}
```

### Tool use is end-to-end (not just a schema)

The SDK ships the pieces you need to build a complete tool loop:
- create tool schemas from types
- parse/validate tool args into typed structs
- execute tool calls via a registry
- feed results back as tool result messages
- retry tool calls when args are malformed (with model-facing error formatting)

```rust
use modelrelay::{
    function_tool_from_type, parse_tool_args, respond_to_tool_call_json, ResponseBuilder, Tool,
    ToolChoice, ToolRegistry, ResponseExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct WeatherArgs {
    location: String,
}

let registry = ToolRegistry::new().register(
    "get_weather",
    modelrelay::sync_handler(|_args_json, call| {
        let args: WeatherArgs = parse_tool_args(call)?;
        Ok(serde_json::json!({ "location": args.location, "temp_f": 72 }))
    }),
);

let schema = function_tool_from_type::<WeatherArgs>()?;
let tool = Tool::function(
    "get_weather",
    Some("Get current weather for a location".into()),
    Some(schema.parameters),
);

let response = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Use the tool to get the weather in San Francisco.")
    .tools(vec![tool])
    .tool_choice(ToolChoice::auto())
    .send(&client.responses())
    .await?;

if response.has_tool_calls() {
    let call = response.first_tool_call().unwrap();
    let result = registry.execute(call).await;
    let tool_result = respond_to_tool_call_json(call, &result.result)?;

    // Feed the tool result back as an input item and continue the conversation.
    let followup = ResponseBuilder::new()
        .model("claude-sonnet-4-20250514")
        .user("Great—now summarize it in one sentence.")
        .item(tool_result)
        .send(&client.responses())
        .await?;

    println!("followup tokens: {}", followup.usage.total());
}
```

### tools.v0 local filesystem tools (fs.*)

The Rust SDK includes a safe-by-default local filesystem tool pack that implements:
`fs.read_file`, `fs.list_files`, and `fs.search`.

```rust
use modelrelay::{LocalFSToolPack, ToolRegistry};

let mut registry = ToolRegistry::new();
let fs_tools = LocalFSToolPack::new(".", Vec::new());
fs_tools.register_into(&mut registry);

// Now registry can execute fs.read_file/fs.list_files/fs.search tool calls.
```

## Customer-Attributed Requests

For metered billing, set `customer_id(...)`. The customer’s tier can determine the model (so `model(...)` can be omitted):

```rust
use modelrelay::ResponseBuilder;

let response = ResponseBuilder::new()
    .customer_id("customer-123")
    .user("Hello!")
    .send(&client.responses())
    .await?;
```

## Blocking API (No Tokio)

Enable the `blocking` feature and use the same builder ergonomics:

```rust
use modelrelay::{BlockingClient, BlockingConfig, ResponseBuilder};

let client = BlockingClient::new(BlockingConfig {
    api_key: Some(std::env::var("MODELRELAY_API_KEY")?),
    ..Default::default()
})?;

let response = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Hello!")
    .send_blocking(&client.responses())?;
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `streaming` | Yes | NDJSON streaming support |
| `blocking` | No | Sync client without Tokio |
| `tracing` | No | OpenTelemetry spans/events |
| `mock` | No | In-memory client for tests |

## Errors

Errors are typed so callers can branch cleanly:

```rust
use modelrelay::{Error, ResponseBuilder};

let result = ResponseBuilder::new()
    .model("claude-sonnet-4-20250514")
    .user("Hello!")
    .send(&client.responses())
    .await;

match result {
    Ok(_response) => {}
    Err(Error::Api(e)) if e.is_rate_limit() => {}
    Err(Error::Api(e)) if e.is_unauthorized() => {}
    Err(Error::Transport(_)) => {}
    Err(e) => return Err(e.into()),
}
```
