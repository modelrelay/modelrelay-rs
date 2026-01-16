use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::convenience::{AgentUsage, DEFAULT_MAX_TURNS};
use crate::errors::{Error, Result, ValidationError};
use crate::generated::{SqlPolicy, SqlValidateRequest};
use crate::responses::{get_all_tool_calls_from_response, ResponseBuilder};
use crate::tools::{assistant_message_with_tool_calls, BoxFuture, ToolBuilder};
use crate::types::{InputItem, Model};
use crate::Client;

pub type SqlRow = serde_json::Map<String, Value>;

/// Type alias for sample rows handler to satisfy clippy::type_complexity.
pub type SampleRowsHandler = Arc<
    dyn Fn(SqlSampleRowsArgs) -> BoxFuture<'static, Result<SqlExecuteResult, String>> + Send + Sync,
>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqlTableInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqlColumnInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nullable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqlTableDescription {
    pub table: String,
    pub columns: Vec<SqlColumnInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqlExecuteResult {
    pub columns: Vec<String>,
    pub rows: Vec<SqlRow>,
}

#[derive(Clone)]
pub struct SqlToolLoopHandlers {
    pub list_tables:
        Arc<dyn Fn() -> BoxFuture<'static, Result<Vec<SqlTableInfo>, String>> + Send + Sync>,
    pub describe_table: Arc<
        dyn Fn(SqlDescribeTableArgs) -> BoxFuture<'static, Result<SqlTableDescription, String>>
            + Send
            + Sync,
    >,
    pub sample_rows: Option<SampleRowsHandler>,
    pub execute_sql: Arc<
        dyn Fn(SqlExecuteArgs) -> BoxFuture<'static, Result<SqlExecuteResult, String>>
            + Send
            + Sync,
    >,
}

impl SqlToolLoopHandlers {
    pub fn new<L, D, E>(list_tables: L, describe_table: D, execute_sql: E) -> Self
    where
        L: Fn() -> BoxFuture<'static, Result<Vec<SqlTableInfo>, String>> + Send + Sync + 'static,
        D: Fn(SqlDescribeTableArgs) -> BoxFuture<'static, Result<SqlTableDescription, String>>
            + Send
            + Sync
            + 'static,
        E: Fn(SqlExecuteArgs) -> BoxFuture<'static, Result<SqlExecuteResult, String>>
            + Send
            + Sync
            + 'static,
    {
        Self {
            list_tables: Arc::new(list_tables),
            describe_table: Arc::new(describe_table),
            sample_rows: None,
            execute_sql: Arc::new(execute_sql),
        }
    }

    pub fn new_sync<L, D, E>(list_tables: L, describe_table: D, execute_sql: E) -> Self
    where
        L: Fn() -> Result<Vec<SqlTableInfo>, String> + Send + Sync + 'static,
        D: Fn(SqlDescribeTableArgs) -> Result<SqlTableDescription, String> + Send + Sync + 'static,
        E: Fn(SqlExecuteArgs) -> Result<SqlExecuteResult, String> + Send + Sync + 'static,
    {
        let list_tables = Arc::new(list_tables);
        let describe_table = Arc::new(describe_table);
        let execute_sql = Arc::new(execute_sql);
        Self::new(
            move || {
                let list_tables = list_tables.clone();
                Box::pin(async move { list_tables() })
            },
            move |args| {
                let describe_table = describe_table.clone();
                Box::pin(async move { describe_table(args) })
            },
            move |args| {
                let execute_sql = execute_sql.clone();
                Box::pin(async move { execute_sql(args) })
            },
        )
    }

    pub fn with_sample_rows<F>(mut self, handler: F) -> Self
    where
        F: Fn(SqlSampleRowsArgs) -> BoxFuture<'static, Result<SqlExecuteResult, String>>
            + Send
            + Sync
            + 'static,
    {
        self.sample_rows = Some(Arc::new(handler));
        self
    }

    pub fn with_sample_rows_sync<F>(mut self, handler: F) -> Self
    where
        F: Fn(SqlSampleRowsArgs) -> Result<SqlExecuteResult, String> + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        self.sample_rows = Some(Arc::new(move |args| {
            let handler = handler.clone();
            Box::pin(async move { handler(args) })
        }));
        self
    }
}

#[derive(Debug, Clone)]
pub struct SqlToolLoopOptions {
    pub model: Model,
    pub prompt: String,
    pub system: Option<String>,
    pub policy: Option<SqlPolicy>,
    pub profile_id: Option<uuid::Uuid>,
    pub max_attempts: Option<usize>,
    pub require_schema_inspection: Option<bool>,
    pub sample_rows: Option<bool>,
    pub sample_rows_limit: Option<usize>,
    pub result_limit: Option<usize>,
}

impl SqlToolLoopOptions {
    pub fn new(model: impl Into<Model>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
            system: None,
            policy: None,
            profile_id: None,
            max_attempts: None,
            require_schema_inspection: None,
            sample_rows: None,
            sample_rows_limit: None,
            result_limit: None,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_policy(mut self, policy: SqlPolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    pub fn with_profile_id(mut self, profile_id: uuid::Uuid) -> Self {
        self.profile_id = Some(profile_id);
        self
    }

    pub fn with_max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = Some(max_attempts);
        self
    }

    pub fn with_sample_rows(mut self, enabled: bool) -> Self {
        self.sample_rows = Some(enabled);
        self
    }

    pub fn with_sample_rows_limit(mut self, limit: usize) -> Self {
        self.sample_rows_limit = Some(limit);
        self
    }

    pub fn with_result_limit(mut self, limit: usize) -> Self {
        self.result_limit = Some(limit);
        self
    }

    pub fn with_require_schema_inspection(mut self, enabled: bool) -> Self {
        self.require_schema_inspection = Some(enabled);
        self
    }
}

#[derive(Clone)]
pub struct SqlToolLoopBuilder {
    client: Client,
    options: SqlToolLoopOptions,
    handlers: SqlToolLoopHandlers,
}

impl SqlToolLoopBuilder {
    pub fn new(
        client: Client,
        model: impl Into<Model>,
        prompt: impl Into<String>,
        handlers: SqlToolLoopHandlers,
    ) -> Self {
        Self {
            client,
            options: SqlToolLoopOptions::new(model, prompt),
            handlers,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.options.system = Some(system.into());
        self
    }

    pub fn with_policy(mut self, policy: SqlPolicy) -> Self {
        self.options.policy = Some(policy);
        self
    }

    pub fn with_profile_id(mut self, profile_id: uuid::Uuid) -> Self {
        self.options.profile_id = Some(profile_id);
        self
    }

    pub fn with_max_attempts(mut self, max_attempts: usize) -> Self {
        self.options.max_attempts = Some(max_attempts);
        self
    }

    pub fn with_sample_rows(mut self, enabled: bool) -> Self {
        self.options.sample_rows = Some(enabled);
        self
    }

    pub fn with_sample_rows_limit(mut self, limit: usize) -> Self {
        self.options.sample_rows_limit = Some(limit);
        self
    }

    pub fn with_result_limit(mut self, limit: usize) -> Self {
        self.options.result_limit = Some(limit);
        self
    }

    pub fn with_require_schema_inspection(mut self, enabled: bool) -> Self {
        self.options.require_schema_inspection = Some(enabled);
        self
    }

    pub fn options(&self) -> &SqlToolLoopOptions {
        &self.options
    }

    pub async fn run(self) -> Result<SqlToolLoopResult> {
        self.client.sql_tool_loop(self.options, self.handlers).await
    }
}

#[derive(Debug, Clone)]
pub struct SqlToolLoopResult {
    pub summary: String,
    pub sql: String,
    pub columns: Vec<String>,
    pub rows: Vec<SqlRow>,
    pub usage: AgentUsage,
    pub attempts: usize,
    pub notes: Option<String>,
}

const DEFAULT_MAX_ATTEMPTS: usize = 3;
const DEFAULT_SAMPLE_ROWS_LIMIT: usize = 3;
const MAX_SAMPLE_ROWS_LIMIT: usize = 10;
const DEFAULT_RESULT_LIMIT: usize = 100;
const MAX_RESULT_LIMIT: usize = 1000;

#[derive(Debug, Clone)]
struct SqlToolLoopConfig {
    max_attempts: usize,
    result_limit: usize,
    sample_rows_limit: usize,
    require_schema_inspection: bool,
    sample_rows_enabled: bool,
    profile_id: Option<uuid::Uuid>,
    policy: Option<SqlPolicy>,
}

#[derive(Debug, Clone)]
struct SqlToolLoopState {
    attempts: usize,
    list_tables_called: bool,
    described_tables: HashSet<String>,
    last_sql: String,
    last_columns: Vec<String>,
    last_rows: Vec<SqlRow>,
    last_notes: String,
}

impl SqlToolLoopState {
    fn new() -> Self {
        Self {
            attempts: 0,
            list_tables_called: false,
            described_tables: HashSet::new(),
            last_sql: String::new(),
            last_columns: Vec::new(),
            last_rows: Vec::new(),
            last_notes: String::new(),
        }
    }

    fn to_result(&self, summary: String, usage: AgentUsage) -> SqlToolLoopResult {
        let notes = if !self.last_notes.is_empty() {
            Some(self.last_notes.clone())
        } else if self.last_sql.is_empty() {
            Some("no SQL executed".to_string())
        } else {
            None
        };
        SqlToolLoopResult {
            summary,
            sql: self.last_sql.clone(),
            columns: self.last_columns.clone(),
            rows: self.last_rows.clone(),
            usage,
            attempts: self.attempts,
            notes,
        }
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct EmptyArgs {}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct DescribeTableArgs {
    table: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct SampleRowsArgs {
    table: String,
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct ExecuteSqlArgs {
    query: String,
    limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SqlDescribeTableArgs {
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct SqlSampleRowsArgs {
    pub table: String,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct SqlExecuteArgs {
    pub query: String,
    pub limit: usize,
}

fn validate_sql_tool_loop_options(
    opts: &SqlToolLoopOptions,
    handlers: &SqlToolLoopHandlers,
) -> Result<()> {
    if opts.prompt.trim().is_empty() {
        return Err(Error::Validation(
            ValidationError::new("prompt is required").with_field("prompt"),
        ));
    }
    if opts.model.is_empty() {
        return Err(Error::Validation(
            ValidationError::new("model is required").with_field("model"),
        ));
    }
    if opts.policy.is_none() && opts.profile_id.is_none() {
        return Err(Error::Validation(
            ValidationError::new("policy or profile_id is required")
                .with_field("policy")
                .with_field("profile_id"),
        ));
    }
    if opts.sample_rows == Some(true) && handlers.sample_rows.is_none() {
        return Err(Error::Validation(
            ValidationError::new("sample_rows handler is required when sample_rows is enabled")
                .with_field("sample_rows"),
        ));
    }
    Ok(())
}

fn normalize_sql_tool_loop_config(
    opts: &SqlToolLoopOptions,
    handlers: &SqlToolLoopHandlers,
) -> SqlToolLoopConfig {
    let max_attempts = opts.max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS).max(1);
    let require_schema_inspection = opts.require_schema_inspection.unwrap_or(true);
    let sample_rows_enabled = opts.sample_rows.unwrap_or(handlers.sample_rows.is_some());
    SqlToolLoopConfig {
        max_attempts,
        result_limit: clamp_limit(opts.result_limit, DEFAULT_RESULT_LIMIT, MAX_RESULT_LIMIT),
        sample_rows_limit: clamp_limit(
            opts.sample_rows_limit,
            DEFAULT_SAMPLE_ROWS_LIMIT,
            MAX_SAMPLE_ROWS_LIMIT,
        ),
        require_schema_inspection,
        sample_rows_enabled,
        profile_id: opts.profile_id,
        policy: opts.policy.clone(),
    }
}

fn build_sql_tool_loop_tools(
    cfg: &SqlToolLoopConfig,
    state: Arc<tokio::sync::Mutex<SqlToolLoopState>>,
    handlers: &SqlToolLoopHandlers,
    sql_client: crate::SqlClient,
) -> ToolBuilder {
    let mut tools = ToolBuilder::new();

    let state_list = state.clone();
    let list_tables = handlers.list_tables.clone();
    tools = tools.add_async::<EmptyArgs, _>(
        "list_tables",
        "List available tables in the database.",
        move |_args, _call| {
            let list_tables = list_tables.clone();
            let state_list = state_list.clone();
            Box::pin(async move {
                let mut locked = state_list.lock().await;
                locked.list_tables_called = true;
                drop(locked);
                let tables = list_tables().await?;
                serde_json::to_value(tables).map_err(|err| err.to_string())
            })
        },
    );

    let state_desc = state.clone();
    let describe_table = handlers.describe_table.clone();
    tools = tools.add_async::<DescribeTableArgs, _>(
        "describe_table",
        "Describe a table's columns and types.",
        move |args, _call| {
            let describe_table = describe_table.clone();
            let state_desc = state_desc.clone();
            Box::pin(async move {
                if args.table.trim().is_empty() {
                    return Err("describe_table requires table".to_string());
                }
                let table_name = normalize_table_name(&args.table);
                let mut locked = state_desc.lock().await;
                locked.described_tables.insert(table_name);
                drop(locked);
                let result = describe_table(SqlDescribeTableArgs { table: args.table }).await?;
                serde_json::to_value(result).map_err(|err| err.to_string())
            })
        },
    );

    if cfg.sample_rows_enabled {
        if let Some(sample_rows) = handlers.sample_rows.clone() {
            let cfg = cfg.clone();
            tools = tools.add_async::<SampleRowsArgs, _>(
                "sample_rows",
                "Return a small sample of rows from a table.",
                move |args, _call| {
                    let sample_rows = sample_rows.clone();
                    Box::pin(async move {
                        if args.table.trim().is_empty() {
                            return Err("sample_rows requires table".to_string());
                        }
                        let limit = clamp_limit(
                            args.limit.map(|v| v as usize),
                            cfg.sample_rows_limit,
                            cfg.sample_rows_limit,
                        );
                        let result = sample_rows(SqlSampleRowsArgs {
                            table: args.table,
                            limit,
                        })
                        .await?;
                        serde_json::to_value(result).map_err(|err| err.to_string())
                    })
                },
            );
        }
    }

    let state_exec = state.clone();
    let execute_sql = handlers.execute_sql.clone();
    let cfg = cfg.clone();
    tools = tools.add_async::<ExecuteSqlArgs, _>(
        "execute_sql",
        "Execute a read-only SQL query against the database.",
        move |args, _call| {
            let execute_sql = execute_sql.clone();
            let state_exec = state_exec.clone();
            let sql_client = sql_client.clone();
            let cfg = cfg.clone();
            Box::pin(async move {
                if args.query.trim().is_empty() {
                    return Err("execute_sql requires query".to_string());
                }

                {
                    let locked = state_exec.lock().await;
                    if locked.attempts >= cfg.max_attempts {
                        return Err("max_attempts exceeded for execute_sql".to_string());
                    }
                }

                let limit = clamp_limit(
                    args.limit.map(|v| v as usize),
                    cfg.result_limit,
                    cfg.result_limit,
                );
                let validate_req = SqlValidateRequest {
                    sql: args.query.clone(),
                    profile_id: cfg.profile_id,
                    policy: cfg.policy.clone(),
                    overrides: None,
                };
                if validate_req.profile_id.is_none() && validate_req.policy.is_none() {
                    return Err("policy or profile_id is required".to_string());
                }

                let validation = sql_client
                    .validate(validate_req)
                    .await
                    .map_err(|err| format!("sql.validate failed: {err}"))?;

                if !validation.valid {
                    return Err("sql.validate rejected query".to_string());
                }
                if !validation.read_only {
                    return Err("sql.validate rejected query: read_only=false".to_string());
                }
                if validation.normalized_sql.trim().is_empty() {
                    return Err("sql.validate rejected query: missing normalized_sql".to_string());
                }

                if cfg.require_schema_inspection {
                    let locked = state_exec.lock().await;
                    if !locked.list_tables_called {
                        return Err("list_tables must be called before execute_sql".to_string());
                    }
                    let mut missing = Vec::new();
                    for table in validation.tables.iter() {
                        let normalized = normalize_table_name(table);
                        if !locked.described_tables.contains(&normalized) {
                            missing.push(table.clone());
                        }
                    }
                    if !missing.is_empty() {
                        return Err(format!(
                            "describe_table required for: {}",
                            missing.join(", ")
                        ));
                    }
                }

                {
                    let mut locked = state_exec.lock().await;
                    locked.attempts += 1;
                    locked.last_sql = validation.normalized_sql.clone();
                }

                let result = execute_sql(SqlExecuteArgs {
                    query: validation.normalized_sql,
                    limit,
                })
                .await?;

                let mut locked = state_exec.lock().await;
                locked.last_columns = result.columns.clone();
                locked.last_rows = result.rows.clone();
                if locked.last_rows.is_empty() {
                    locked.last_notes = "query returned no rows".to_string();
                } else {
                    locked.last_notes.clear();
                }

                serde_json::to_value(result).map_err(|err| err.to_string())
            })
        },
    );

    tools
}

fn clamp_limit(value: Option<usize>, fallback: usize, max: usize) -> usize {
    let v = value.unwrap_or(fallback);
    if v == 0 {
        fallback
    } else {
        v.min(max)
    }
}

fn normalize_table_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn sql_loop_system_prompt(
    max_attempts: usize,
    result_limit: usize,
    sample_rows: bool,
    require_schema_inspection: bool,
    extra: Option<&str>,
) -> String {
    let mut steps = vec![
        "Use list_tables to see available tables.".to_string(),
        "Use describe_table on any table you query.".to_string(),
    ];
    if sample_rows {
        steps.push("Use sample_rows for quick context if needed.".to_string());
    }
    steps.push("Generate a read-only SELECT query.".to_string());
    steps.push("Call execute_sql to run it.".to_string());
    let mut lines = vec![
        "You are a SQL assistant that must follow this workflow:".to_string(),
        format!("- {}", steps.join("\n- ")),
        format!("- Maximum SQL attempts: {max_attempts}."),
        format!("- Always keep result size <= {result_limit} rows."),
    ];
    if require_schema_inspection {
        lines.push("- Do not execute SQL until schema inspection is complete.".to_string());
    } else {
        lines.push("- Schema inspection is optional but recommended.".to_string());
    }
    lines.push("Return a concise summary of the results when done.".to_string());
    if let Some(extra) = extra {
        if !extra.trim().is_empty() {
            lines.push(String::new());
            lines.push(extra.trim().to_string());
        }
    }
    lines.join("\n")
}

impl Client {
    pub async fn sql_tool_loop(
        &self,
        opts: SqlToolLoopOptions,
        handlers: SqlToolLoopHandlers,
    ) -> Result<SqlToolLoopResult> {
        validate_sql_tool_loop_options(&opts, &handlers)?;
        let cfg = normalize_sql_tool_loop_config(&opts, &handlers);
        let state = Arc::new(tokio::sync::Mutex::new(SqlToolLoopState::new()));

        let tools_builder = build_sql_tool_loop_tools(&cfg, state.clone(), &handlers, self.sql());
        let (tool_defs, tool_registry) = tools_builder.build();

        let system_prompt = sql_loop_system_prompt(
            cfg.max_attempts,
            cfg.result_limit,
            cfg.sample_rows_enabled,
            cfg.require_schema_inspection,
            opts.system.as_deref(),
        );

        let mut input = Vec::new();
        if !system_prompt.trim().is_empty() {
            input.push(InputItem::system(system_prompt));
        }
        input.push(InputItem::user(opts.prompt));

        let mut usage = AgentUsage::default();
        let mut last_response = None;

        for _turn in 0..DEFAULT_MAX_TURNS {
            let mut builder = ResponseBuilder::new()
                .model(opts.model.clone())
                .input(input.clone());
            if !tool_defs.is_empty() {
                builder = builder.tools(tool_defs.clone());
            }
            let response = builder.send(&self.responses()).await?;
            usage.llm_calls += 1;
            usage.input_tokens += response.usage.input_tokens as u64;
            usage.output_tokens += response.usage.output_tokens as u64;
            usage.total_tokens += response.usage.total_tokens as u64;
            last_response = Some(response.clone());

            let tool_calls = get_all_tool_calls_from_response(&response);
            if tool_calls.is_empty() {
                let summary = response.text();
                let locked = state.lock().await;
                return Ok(locked.to_result(summary, usage));
            }

            usage.tool_calls += tool_calls.len();
            input.push(assistant_message_with_tool_calls(
                response.text(),
                tool_calls.clone(),
            ));
            let results = tool_registry.execute_all(&tool_calls).await;
            let messages = tool_registry.results_to_messages(&results);
            input.extend(messages);
        }

        let summary = last_response.map(|r| r.text()).unwrap_or_default();
        let locked = state.lock().await;
        Ok(locked.to_result(summary, usage))
    }

    pub fn sql_tool_loop_builder(
        &self,
        model: impl Into<Model>,
        prompt: impl Into<String>,
        handlers: SqlToolLoopHandlers,
    ) -> SqlToolLoopBuilder {
        SqlToolLoopBuilder::new(self.clone(), model, prompt, handlers)
    }

    pub async fn sql_tool_loop_quickstart(
        &self,
        model: impl Into<Model>,
        prompt: impl Into<String>,
        handlers: SqlToolLoopHandlers,
        policy: Option<SqlPolicy>,
        profile_id: Option<uuid::Uuid>,
    ) -> Result<SqlToolLoopResult> {
        let mut opts = SqlToolLoopOptions::new(model, prompt);
        opts.policy = policy;
        opts.profile_id = profile_id;
        self.sql_tool_loop(opts, handlers).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::test_client;
    use crate::types::{ToolCall, ToolType};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_normalize_sql_tool_loop_config_disables_sample_rows_without_handler() {
        let handlers = SqlToolLoopHandlers::new_sync(
            || Ok(vec![]),
            |_args| {
                Ok(SqlTableDescription {
                    table: "t".to_string(),
                    columns: vec![],
                })
            },
            |_args| {
                Ok(SqlExecuteResult {
                    columns: vec![],
                    rows: vec![],
                })
            },
        );
        let opts = SqlToolLoopOptions::new("model", "prompt");
        let cfg = normalize_sql_tool_loop_config(&opts, &handlers);
        assert!(!cfg.sample_rows_enabled);
    }

    #[test]
    fn test_sql_tool_loop_builder_sets_options() {
        let client = test_client("http://example.com");
        let handlers = SqlToolLoopHandlers::new_sync(
            || Ok(vec![]),
            |_args| {
                Ok(SqlTableDescription {
                    table: "t".to_string(),
                    columns: vec![],
                })
            },
            |_args| {
                Ok(SqlExecuteResult {
                    columns: vec![],
                    rows: vec![],
                })
            },
        );
        let profile_id = uuid::Uuid::new_v4();
        let builder = client
            .sql_tool_loop_builder("model", "prompt", handlers)
            .with_profile_id(profile_id)
            .with_max_attempts(5);
        let options = builder.options();
        assert_eq!(options.profile_id, Some(profile_id));
        assert_eq!(options.max_attempts, Some(5));
    }

    #[tokio::test]
    async fn test_execute_sql_rejects_non_read_only_validation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sql/validate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "valid": true,
                "read_only": false,
                "normalized_sql": "DELETE FROM users"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let executed = Arc::new(AtomicBool::new(false));
        let executed_flag = executed.clone();

        let handlers = SqlToolLoopHandlers::new_sync(
            || Ok(vec![]),
            |_args| {
                Ok(SqlTableDescription {
                    table: "t".to_string(),
                    columns: vec![],
                })
            },
            move |_args| {
                executed_flag.store(true, Ordering::SeqCst);
                Ok(SqlExecuteResult {
                    columns: vec![],
                    rows: vec![],
                })
            },
        );

        let opts = SqlToolLoopOptions::new("model", "prompt")
            .with_profile_id(uuid::Uuid::new_v4())
            .with_require_schema_inspection(false);
        let cfg = normalize_sql_tool_loop_config(&opts, &handlers);
        let state = Arc::new(tokio::sync::Mutex::new(SqlToolLoopState::new()));
        let tools = build_sql_tool_loop_tools(&cfg, state, &handlers, client.sql());
        let (_defs, registry) = tools.build();

        let call = ToolCall {
            id: "call-1".to_string(),
            kind: ToolType::Function,
            function: Some(crate::types::FunctionCall {
                name: "execute_sql".to_string(),
                arguments: r#"{"query":"DELETE FROM users"}"#.to_string(),
            }),
        };
        let result = registry.execute(&call).await;
        assert!(result.error.is_some());
        assert!(!executed.load(Ordering::SeqCst));
    }
}
