//! Plugin helpers for converting GitHub-hosted plugins into workflows.
//!
//! Plugins are markdown files stored in GitHub repos. The SDK loads the plugin,
//! converts commands to workflow specs using `/responses`, and executes them via `/runs`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{StatusCode, Url};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::errors::Error as SdkError;
use crate::structured::{output_format_from_type, StructuredError};
use crate::tools::{ToolExecutionResult, ToolRegistry};
use crate::types::{FunctionCall, Model, OutputFormat, ToolCall, ToolType};
use crate::workflow::events::NodeWaitingV0;
use crate::workflow::{ModelId, RunCostSummaryV0, RunEventV0, RunId, RunStatusV0};
use crate::workflow_intent::{
    WorkflowIntentKind, WorkflowIntentNode, WorkflowIntentNodeType, WorkflowIntentOutputRef,
    WorkflowIntentSpec, WorkflowIntentToolExecution, WorkflowIntentToolExecutionMode,
    WorkflowIntentToolRef,
};
use crate::{
    Client, ResponseBuilder, RunsCreateOptions, RunsToolCallV0, RunsToolResultItemV0, ToolCallId,
    ToolName,
};

const DEFAULT_PLUGIN_REF: &str = "HEAD";
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";
const DEFAULT_GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com";
const DEFAULT_CONVERTER_MODEL: &str = "claude-3-5-haiku-latest";

const DEFAULT_DYNAMIC_TOOLS: &[PluginToolName] = &[
    PluginToolName::FsReadFile,
    PluginToolName::FsListFiles,
    PluginToolName::FsSearch,
];

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(value: impl Into<String>) -> Result<Self, PluginError> {
        let trimmed = value.into().trim().to_string();
        if trimmed.is_empty() {
            return Err(PluginError::Loader("plugin id is required".to_string()));
        }
        Ok(Self(trimmed))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<PluginId> for String {
    fn from(value: PluginId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PluginUrl(String);

impl PluginUrl {
    pub fn new(value: impl Into<String>) -> Result<Self, PluginError> {
        let trimmed = value.into().trim().to_string();
        if trimmed.is_empty() {
            return Err(PluginError::Loader("plugin url is required".to_string()));
        }
        Ok(Self(trimmed))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PluginUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<PluginUrl> for String {
    fn from(value: PluginUrl) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PluginCommandName(String);

impl PluginCommandName {
    pub fn new(value: impl Into<String>) -> Result<Self, PluginError> {
        let trimmed = value.into().trim().to_string();
        if trimmed.is_empty() {
            return Err(PluginError::Loader(
                "plugin command name is required".to_string(),
            ));
        }
        Ok(Self(trimmed))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PluginCommandName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<PluginCommandName> for String {
    fn from(value: PluginCommandName) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PluginAgentName(String);

impl PluginAgentName {
    pub fn new(value: impl Into<String>) -> Result<Self, PluginError> {
        let trimmed = value.into().trim().to_string();
        if trimmed.is_empty() {
            return Err(PluginError::Loader(
                "plugin agent name is required".to_string(),
            ));
        }
        Ok(Self(trimmed))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PluginAgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<PluginAgentName> for String {
    fn from(value: PluginAgentName) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginToolName {
    #[serde(rename = "fs.read_file")]
    FsReadFile,
    #[serde(rename = "fs.list_files")]
    FsListFiles,
    #[serde(rename = "fs.search")]
    FsSearch,
    #[serde(rename = "fs.edit")]
    FsEdit,
    #[serde(rename = "bash")]
    Bash,
    #[serde(rename = "write_file")]
    WriteFile,
    #[serde(rename = "user.ask")]
    UserAsk,
}

impl PluginToolName {
    pub fn as_str(&self) -> &'static str {
        match self {
            PluginToolName::FsReadFile => "fs.read_file",
            PluginToolName::FsListFiles => "fs.list_files",
            PluginToolName::FsSearch => "fs.search",
            PluginToolName::FsEdit => "fs.edit",
            PluginToolName::Bash => "bash",
            PluginToolName::WriteFile => "write_file",
            PluginToolName::UserAsk => "user.ask",
        }
    }
}

impl fmt::Display for PluginToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for PluginToolName {
    type Err = PluginOrchestrationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "fs.read_file" => Ok(PluginToolName::FsReadFile),
            "fs.list_files" => Ok(PluginToolName::FsListFiles),
            "fs.search" => Ok(PluginToolName::FsSearch),
            "fs.edit" => Ok(PluginToolName::FsEdit),
            "bash" => Ok(PluginToolName::Bash),
            "write_file" => Ok(PluginToolName::WriteFile),
            "user.ask" => Ok(PluginToolName::UserAsk),
            other => Err(PluginOrchestrationError::new(
                PluginOrchestrationErrorCode::UnknownTool,
                format!("unknown tool \"{}\"", other),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrchestrationMode {
    #[default]
    Dag,
    Dynamic,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub commands: Option<Vec<PluginCommandName>>,
    pub agents: Option<Vec<PluginAgentName>>,
}

#[derive(Debug, Clone)]
pub struct PluginCommand {
    pub name: PluginCommandName,
    pub prompt: String,
    pub agent_refs: Option<Vec<PluginAgentName>>,
    pub tools: Option<Vec<PluginToolName>>,
}

#[derive(Debug, Clone)]
pub struct PluginAgent {
    pub name: PluginAgentName,
    pub system_prompt: String,
    pub description: Option<String>,
    pub tools: Option<Vec<PluginToolName>>,
}

#[derive(Debug, Clone)]
pub struct PluginGitHubRef {
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub id: PluginId,
    pub url: PluginUrl,
    pub manifest: PluginManifest,
    pub commands: HashMap<PluginCommandName, PluginCommand>,
    pub agents: HashMap<PluginAgentName, PluginAgent>,
    pub raw_files: HashMap<String, String>,
    pub git_ref: PluginGitHubRef,
    pub loaded_at: DateTime<Utc>,
}

#[derive(Clone, Default)]
pub struct PluginRunConfig {
    pub model: Option<ModelId>,
    pub converter_model: Option<Model>,
    pub orchestration_mode: Option<OrchestrationMode>,
    pub user_task: String,
    pub tool_registry: Option<Arc<ToolRegistry>>,
    pub run_options: Option<RunsCreateOptions>,
}

#[derive(Debug, Clone)]
pub struct PluginRunResult {
    pub run_id: RunId,
    pub status: RunStatusV0,
    pub outputs: HashMap<String, Value>,
    pub cost_summary: RunCostSummaryV0,
    pub events: Vec<RunEventV0>,
}

#[derive(Clone, Default)]
pub struct PluginLoaderOptions {
    pub http_client: Option<reqwest::Client>,
    pub api_base_url: Option<String>,
    pub raw_base_url: Option<String>,
    pub cache_ttl: Option<Duration>,
    pub now: Option<Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>>,
}

#[derive(Debug, Clone, Default)]
pub struct PluginConverterOptions {
    pub converter_model: Option<Model>,
}

#[derive(Clone, Default)]
pub struct PluginsClientOptions {
    pub loader: Option<PluginLoaderOptions>,
}

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("{0}")]
    Orchestration(#[from] PluginOrchestrationError),
    #[error("plugin loader: {0}")]
    Loader(String),
    #[error("{0}")]
    Sdk(#[from] SdkError),
    #[error("{0}")]
    Structured(#[from] StructuredError),
    #[error("{0}")]
    Run(#[from] PluginRunError),
    #[error("plugin conversion: {0}")]
    Conversion(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginOrchestrationErrorCode {
    InvalidPlan,
    UnknownAgent,
    MissingDescription,
    UnknownTool,
    InvalidDependency,
    InvalidToolConfig,
}

impl fmt::Display for PluginOrchestrationErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            PluginOrchestrationErrorCode::InvalidPlan => "INVALID_PLAN",
            PluginOrchestrationErrorCode::UnknownAgent => "UNKNOWN_AGENT",
            PluginOrchestrationErrorCode::MissingDescription => "MISSING_DESCRIPTION",
            PluginOrchestrationErrorCode::UnknownTool => "UNKNOWN_TOOL",
            PluginOrchestrationErrorCode::InvalidDependency => "INVALID_DEPENDENCY",
            PluginOrchestrationErrorCode::InvalidToolConfig => "INVALID_TOOL_CONFIG",
        };
        write!(f, "{}", value)
    }
}

#[derive(Debug, Error, Clone)]
#[error("plugin orchestration: {message}")]
pub struct PluginOrchestrationError {
    pub code: PluginOrchestrationErrorCode,
    pub message: String,
}

impl PluginOrchestrationError {
    pub fn new(code: PluginOrchestrationErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
#[error("plugin run {status} ({run_id})")]
pub struct PluginRunError {
    pub run_id: RunId,
    pub status: RunStatusV0,
    pub events: Vec<RunEventV0>,
}

#[derive(Debug, Clone)]
struct GitHubPluginRef {
    owner: String,
    repo: String,
    git_ref: String,
    repo_path: String,
    canonical: String,
}

#[derive(Debug, Clone)]
struct CachedPlugin {
    expires_at: DateTime<Utc>,
    plugin: Plugin,
}

#[derive(Debug, Deserialize)]
struct GitHubContentEntry {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    path: String,
}

pub struct PluginLoader {
    http: reqwest::Client,
    api_base_url: String,
    raw_base_url: String,
    cache_ttl: Duration,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    cache: Mutex<HashMap<String, CachedPlugin>>,
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginLoader {
    pub fn new() -> Self {
        Self::with_options(PluginLoaderOptions::default())
    }

    pub fn with_options(options: PluginLoaderOptions) -> Self {
        Self {
            http: options.http_client.unwrap_or_default(),
            api_base_url: options
                .api_base_url
                .unwrap_or_else(|| DEFAULT_GITHUB_API_BASE.to_string()),
            raw_base_url: options
                .raw_base_url
                .unwrap_or_else(|| DEFAULT_GITHUB_RAW_BASE.to_string()),
            cache_ttl: options.cache_ttl.unwrap_or(DEFAULT_CACHE_TTL),
            now: options.now.unwrap_or_else(|| Arc::new(Utc::now)),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self, source_url: impl AsRef<str>) -> Result<Plugin, PluginError> {
        let ref_info = parse_github_plugin_ref(source_url.as_ref())?;
        let key = ref_info.canonical.clone();
        if let Some(cached) = self.cached(&key) {
            return Ok(cached);
        }

        let manifest_candidates = ["PLUGIN.md", "SKILL.md"];
        let mut manifest_path = None;
        let mut manifest_body = String::new();
        for name in manifest_candidates {
            let path = join_repo_path(&ref_info.repo_path, name);
            match self.fetch_text(&self.raw_url(&ref_info, &path)).await {
                Ok(body) => {
                    manifest_path = Some(path);
                    manifest_body = body;
                    break;
                }
                Err(err) => {
                    if err.is_not_found() {
                        continue;
                    }
                    return Err(PluginError::Loader(format!("fetch {}: {}", name, err)));
                }
            }
        }
        let manifest_path = manifest_path
            .ok_or_else(|| PluginError::Loader("plugin manifest not found".to_string()))?;

        let commands_dir = join_repo_path(&ref_info.repo_path, "commands");
        let agents_dir = join_repo_path(&ref_info.repo_path, "agents");

        let command_files = self.list_markdown_files(&ref_info, &commands_dir).await?;
        let agent_files = self.list_markdown_files(&ref_info, &agents_dir).await?;

        let mut plugin = Plugin {
            id: PluginId::new(derive_plugin_id(
                &ref_info.owner,
                &ref_info.repo,
                &ref_info.repo_path,
            ))?,
            url: PluginUrl::new(ref_info.canonical.clone())?,
            manifest: parse_plugin_manifest(&manifest_body),
            commands: HashMap::new(),
            agents: HashMap::new(),
            raw_files: HashMap::new(),
            git_ref: PluginGitHubRef {
                owner: ref_info.owner.clone(),
                repo: ref_info.repo.clone(),
                git_ref: ref_info.git_ref.clone(),
                path: if ref_info.repo_path.is_empty() {
                    None
                } else {
                    Some(ref_info.repo_path.clone())
                },
            },
            loaded_at: (self.now)(),
        };

        plugin
            .raw_files
            .insert(manifest_path.clone(), manifest_body.clone());

        for file_path in command_files {
            let body = self
                .fetch_text(&self.raw_url(&ref_info, &file_path))
                .await
                .map_err(|err| PluginError::Loader(format!("fetch {}: {}", file_path, err)))?;
            plugin.raw_files.insert(file_path.clone(), body.clone());
            let parsed = parse_markdown_frontmatter(&body)?;
            let name = PluginCommandName::new(basename(&file_path))?;
            let prompt = parsed.body;
            plugin.commands.insert(
                name.clone(),
                PluginCommand {
                    name,
                    prompt: prompt.clone(),
                    tools: parsed.tools,
                    agent_refs: extract_agent_refs(&prompt),
                },
            );
        }

        for file_path in agent_files {
            let body = self
                .fetch_text(&self.raw_url(&ref_info, &file_path))
                .await
                .map_err(|err| PluginError::Loader(format!("fetch {}: {}", file_path, err)))?;
            plugin.raw_files.insert(file_path.clone(), body.clone());
            let parsed = parse_markdown_frontmatter(&body)?;
            let name = PluginAgentName::new(basename(&file_path))?;
            plugin.agents.insert(
                name.clone(),
                PluginAgent {
                    name,
                    system_prompt: parsed.body,
                    description: parsed.description,
                    tools: parsed.tools,
                },
            );
        }

        plugin.manifest.commands = Some(sorted_keys(plugin.commands.keys().cloned().collect()));
        plugin.manifest.agents = Some(sorted_keys(plugin.agents.keys().cloned().collect()));

        self.store(&key, &plugin);
        Ok(clone_plugin(&plugin))
    }

    fn cached(&self, key: &str) -> Option<Plugin> {
        let guard = self.cache.lock().ok()?;
        let entry = guard.get(key)?;
        if entry.expires_at <= (self.now)() {
            return None;
        }
        Some(clone_plugin(&entry.plugin))
    }

    fn store(&self, key: &str, plugin: &Plugin) {
        if let Ok(mut guard) = self.cache.lock() {
            let expires_at = (self.now)() + chrono::Duration::from_std(self.cache_ttl).unwrap();
            guard.insert(
                key.to_string(),
                CachedPlugin {
                    expires_at,
                    plugin: clone_plugin(plugin),
                },
            );
        }
    }

    async fn list_markdown_files(
        &self,
        ref_info: &GitHubPluginRef,
        repo_dir: &str,
    ) -> Result<Vec<String>, PluginError> {
        let path = format!(
            "{}/repos/{}/{}/contents/{}",
            self.api_base_url.trim_end_matches('/'),
            ref_info.owner,
            ref_info.repo,
            repo_dir.trim_start_matches('/')
        );
        let url = format!("{path}?ref={}", urlencoding::encode(&ref_info.git_ref));
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|err| PluginError::Loader(format!("fetch {}: {}", repo_dir, err)))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !response.status().is_success() {
            return Err(PluginError::Loader(format!(
                "fetch {}: {}",
                repo_dir,
                response.status()
            )));
        }
        let entries = response
            .json::<Vec<GitHubContentEntry>>()
            .await
            .map_err(|err| PluginError::Loader(format!("parse {}: {}", repo_dir, err)))?;
        Ok(entries
            .into_iter()
            .filter(|entry| entry.kind == "file" && entry.name.ends_with(".md"))
            .map(|entry| entry.path)
            .collect())
    }

    fn raw_url(&self, ref_info: &GitHubPluginRef, repo_path: &str) -> String {
        let cleaned = repo_path.trim_start_matches('/');
        format!(
            "{}/{}/{}/{}/{}",
            self.raw_base_url.trim_end_matches('/'),
            ref_info.owner,
            ref_info.repo,
            ref_info.git_ref,
            cleaned
        )
    }

    async fn fetch_text(&self, url: &str) -> Result<String, PluginHTTPError> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|err| PluginHTTPError::new(StatusCode::BAD_GATEWAY, err.to_string()))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|err| {
            PluginHTTPError::new(
                StatusCode::BAD_GATEWAY,
                format!("failed to read response body: {}", err),
            )
        })?;
        if !status.is_success() {
            return Err(PluginHTTPError::new(status, body));
        }
        Ok(body)
    }
}

pub struct PluginConverter {
    client: Client,
    converter_model: Model,
}

impl PluginConverter {
    pub fn new(client: Client, options: PluginConverterOptions) -> Self {
        let model = options
            .converter_model
            .unwrap_or_else(|| Model::from(DEFAULT_CONVERTER_MODEL));
        Self {
            client,
            converter_model: model,
        }
    }

    pub async fn to_workflow(
        &self,
        plugin: &Plugin,
        command_name: impl AsRef<str>,
        task: impl AsRef<str>,
    ) -> Result<WorkflowIntentSpec, PluginError> {
        let command = resolve_command(plugin, command_name.as_ref())?;
        let prompt = build_plugin_conversion_prompt(plugin, command, task.as_ref())?;
        let schema = workflow_intent_schema()?;
        let response = ResponseBuilder::new()
            .model(self.converter_model.clone())
            .system(plugin_to_workflow_system_prompt())
            .user(prompt)
            .output_format(schema)
            .send(&self.client.responses())
            .await?;
        let spec = parse_response_json::<WorkflowIntentSpec>(response)?;
        let mut spec = spec;
        validate_workflow_tools(&mut spec)?;
        Ok(spec)
    }

    pub async fn to_workflow_dynamic(
        &self,
        plugin: &Plugin,
        command_name: impl AsRef<str>,
        task: impl AsRef<str>,
    ) -> Result<WorkflowIntentSpec, PluginError> {
        let command = resolve_command(plugin, command_name.as_ref())?;
        let (candidates, lookup) = build_orchestration_candidates(plugin, command)?;
        let prompt = build_plugin_orchestration_prompt(plugin, command, task.as_ref(), &candidates);
        let output_format =
            output_format_from_type::<OrchestrationPlanV1>(Some("orchestration_plan"))?;
        let response = ResponseBuilder::new()
            .model(self.converter_model.clone())
            .system(plugin_orchestration_system_prompt())
            .user(prompt)
            .output_format(output_format)
            .send(&self.client.responses())
            .await?;
        let plan = parse_response_json::<OrchestrationPlanV1>(response)?;
        validate_orchestration_plan(&plan, &lookup)?;
        let mut spec = build_dynamic_workflow_from_plan(
            plugin,
            command,
            task.as_ref(),
            &plan,
            &lookup,
            &self.converter_model,
        )?;
        if spec_requires_tools(&spec) {
            ensure_model_supports_tools(&self.client, &self.converter_model).await?;
        }
        validate_workflow_tools(&mut spec)?;
        Ok(spec)
    }
}

pub struct PluginRunner {
    client: Client,
}

impl PluginRunner {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn run(
        &self,
        spec: WorkflowIntentSpec,
        config: &PluginRunConfig,
    ) -> Result<PluginRunResult, PluginError> {
        let run_options = config.run_options.clone().unwrap_or_default();
        let created = self
            .client
            .runs()
            .create_with_options(spec, run_options)
            .await?;
        self.wait(created.run_id, config).await
    }

    #[cfg(feature = "streaming")]
    pub async fn wait(
        &self,
        run_id: RunId,
        config: &PluginRunConfig,
    ) -> Result<PluginRunResult, PluginError> {
        let mut events = Vec::new();
        let mut last_seq: Option<i64> = None;
        let mut handled_calls: HashSet<String> = HashSet::new();

        loop {
            let mut stream = self
                .client
                .runs()
                .stream_events(run_id.clone(), last_seq, None)
                .await?;
            use futures_util::StreamExt;
            while let Some(item) = stream.next().await {
                let event = item?;
                last_seq = Some(event.seq() as i64);
                events.push(event.clone());
                match &event.payload {
                    crate::workflow::RunEventPayload::RunCompleted { .. } => {
                        let snapshot = self.client.runs().get(run_id.clone()).await?;
                        return Ok(PluginRunResult {
                            run_id: snapshot.run_id,
                            status: snapshot.status,
                            outputs: snapshot.outputs,
                            cost_summary: snapshot.cost_summary,
                            events,
                        });
                    }
                    crate::workflow::RunEventPayload::RunFailed { .. } => {
                        return Err(PluginRunError {
                            run_id,
                            status: RunStatusV0::Failed,
                            events,
                        }
                        .into());
                    }
                    crate::workflow::RunEventPayload::RunCanceled { .. } => {
                        return Err(PluginRunError {
                            run_id,
                            status: RunStatusV0::Canceled,
                            events,
                        }
                        .into());
                    }
                    crate::workflow::RunEventPayload::NodeWaiting { node_id, waiting } => {
                        let registry = config.tool_registry.clone().ok_or_else(|| {
                            PluginError::Loader(
                                "tool registry required for client tool execution".to_string(),
                            )
                        })?;
                        self.handle_waiting_event(
                            &run_id,
                            node_id,
                            waiting,
                            &registry,
                            &mut handled_calls,
                        )
                        .await?;
                    }
                    crate::workflow::RunEventPayload::NodeToolResult { tool_result, .. } => {
                        handled_calls.insert(tool_result.tool_call.id.to_string());
                    }
                    _ => {}
                }
            }

            let snapshot = self.client.runs().get(run_id.clone()).await?;
            if snapshot.status == RunStatusV0::Succeeded {
                return Ok(PluginRunResult {
                    run_id: snapshot.run_id,
                    status: snapshot.status,
                    outputs: snapshot.outputs,
                    cost_summary: snapshot.cost_summary,
                    events,
                });
            }
            if snapshot.status == RunStatusV0::Failed || snapshot.status == RunStatusV0::Canceled {
                return Err(PluginRunError {
                    run_id,
                    status: snapshot.status,
                    events,
                }
                .into());
            }
        }
    }

    #[cfg(not(feature = "streaming"))]
    pub async fn wait(
        &self,
        run_id: RunId,
        config: &PluginRunConfig,
    ) -> Result<PluginRunResult, PluginError> {
        let mut handled_calls: HashSet<String> = HashSet::new();
        loop {
            let pending = self.client.runs().pending_tools(run_id).await?;
            if let Some(nodes) = pending.pending {
                if !nodes.is_empty() {
                    let registry = config.tool_registry.clone().ok_or_else(|| {
                        PluginError::Loader(
                            "tool registry required for client tool execution".to_string(),
                        )
                    })?;
                    for node in nodes {
                        self.handle_pending_node(&run_id, &node, &registry, &mut handled_calls)
                            .await?;
                    }
                }
            }

            let snapshot = self.client.runs().get(run_id).await?;
            if snapshot.status == RunStatusV0::Succeeded {
                return Ok(PluginRunResult {
                    run_id: snapshot.run_id,
                    status: snapshot.status,
                    outputs: snapshot.outputs,
                    cost_summary: snapshot.cost_summary,
                    events: Vec::new(),
                });
            }
            if snapshot.status == RunStatusV0::Failed || snapshot.status == RunStatusV0::Canceled {
                return Err(PluginRunError {
                    run_id,
                    status: snapshot.status,
                    events: Vec::new(),
                }
                .into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn handle_waiting_event(
        &self,
        run_id: &RunId,
        node_id: &crate::workflow::NodeId,
        waiting: &NodeWaitingV0,
        registry: &ToolRegistry,
        handled: &mut HashSet<String>,
    ) -> Result<(), PluginError> {
        if waiting.pending_tool_calls.is_empty() {
            return Ok(());
        }
        let pending: Vec<PendingToolCallInput> = waiting
            .pending_tool_calls
            .iter()
            .map(|call| PendingToolCallInput {
                id: call.tool_call.id.clone(),
                name: call.tool_call.name.clone(),
                arguments: call.tool_call.arguments.clone(),
            })
            .collect();
        let results = execute_pending_tools(&pending, registry, handled).await?;
        if results.is_empty() {
            return Ok(());
        }
        self.client
            .runs()
            .submit_tool_results(
                run_id.clone(),
                crate::RunsToolResultsRequest {
                    node_id: node_id.clone(),
                    step: waiting.step,
                    request_id: waiting.request_id.clone(),
                    results,
                },
            )
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "streaming"))]
    async fn handle_pending_node(
        &self,
        run_id: &RunId,
        node: &crate::generated::RunsPendingToolsNodeV0,
        registry: &ToolRegistry,
        handled: &mut HashSet<String>,
    ) -> Result<(), PluginError> {
        let pending_calls = node.tool_calls.clone().unwrap_or_default();
        let pending: Vec<PendingToolCallInput> = pending_calls
            .iter()
            .map(|call| PendingToolCallInput {
                id: call.tool_call.id.to_string(),
                name: call.tool_call.name.to_string(),
                arguments: call.tool_call.arguments.clone(),
            })
            .collect();
        let results = execute_pending_tools(&pending, registry, handled).await?;
        if results.is_empty() {
            return Ok(());
        }
        self.client
            .runs()
            .submit_tool_results(
                run_id.clone(),
                crate::RunsToolResultsRequest {
                    node_id: node.node_id.clone(),
                    step: node.step,
                    request_id: node.request_id.clone(),
                    results,
                },
            )
            .await?;
        Ok(())
    }
}

pub struct PluginsClient {
    client: Client,
    loader: PluginLoader,
    runner: PluginRunner,
}

impl PluginsClient {
    pub fn new(client: Client, options: PluginsClientOptions) -> Self {
        let loader = PluginLoader::with_options(options.loader.unwrap_or_default());
        let runner = PluginRunner::new(client.clone());
        Self {
            client,
            loader,
            runner,
        }
    }

    pub async fn load(&self, plugin_url: impl AsRef<str>) -> Result<Plugin, PluginError> {
        let url = plugin_url.as_ref().trim();
        if url.is_empty() {
            return Err(PluginError::Loader("plugin url is required".to_string()));
        }
        self.loader.load(url).await
    }

    pub async fn run(
        &self,
        plugin: &Plugin,
        command: impl AsRef<str>,
        mut config: PluginRunConfig,
    ) -> Result<PluginRunResult, PluginError> {
        let cmd = command.as_ref().trim();
        if cmd.is_empty() {
            return Err(PluginError::Loader("command is required".to_string()));
        }
        config.user_task = config.user_task.trim().to_string();
        if config.user_task.is_empty() {
            return Err(PluginError::Loader("user task is required".to_string()));
        }
        let mode = normalize_orchestration_mode(config.orchestration_mode);
        let converter = if let Some(model) = config.converter_model.clone() {
            PluginConverter::new(
                self.client.clone(),
                PluginConverterOptions {
                    converter_model: Some(model),
                },
            )
        } else {
            PluginConverter::new(self.client.clone(), PluginConverterOptions::default())
        };
        let mut spec = match mode {
            OrchestrationMode::Dynamic => {
                converter
                    .to_workflow_dynamic(plugin, cmd, &config.user_task)
                    .await?
            }
            OrchestrationMode::Dag => {
                converter
                    .to_workflow(plugin, cmd, &config.user_task)
                    .await?
            }
        };
        if let Some(model) = config.model.take() {
            spec.model = Some(model.to_string());
        }
        self.runner.run(spec, &config).await
    }

    pub async fn quick_run(
        &self,
        plugin_url: impl AsRef<str>,
        command: impl AsRef<str>,
        user_task: impl AsRef<str>,
        mut config: PluginRunConfig,
    ) -> Result<PluginRunResult, PluginError> {
        let plugin = self.load(plugin_url).await?;
        config.user_task = user_task.as_ref().to_string();
        self.run(&plugin, command, config).await
    }
}

fn normalize_orchestration_mode(mode: Option<OrchestrationMode>) -> OrchestrationMode {
    mode.unwrap_or_default()
}

fn resolve_command<'a>(plugin: &'a Plugin, name: &str) -> Result<&'a PluginCommand, PluginError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(PluginError::Loader("command is required".to_string()));
    }
    let key = PluginCommandName::new(trimmed.to_string())?;
    plugin
        .commands
        .get(&key)
        .ok_or_else(|| PluginError::Loader("unknown command".to_string()))
}

fn plugin_to_workflow_system_prompt() -> String {
    let tools = [
        PluginToolName::FsReadFile,
        PluginToolName::FsListFiles,
        PluginToolName::FsSearch,
        PluginToolName::FsEdit,
        PluginToolName::Bash,
        PluginToolName::WriteFile,
        PluginToolName::UserAsk,
    ]
    .iter()
    .map(|t| t.as_str())
    .collect::<Vec<_>>()
    .join(", ");

    format!(
        "You convert a ModelRelay plugin (markdown files) into a single workflow JSON spec.\n\nRules:\n- Output MUST be a single JSON object and MUST validate as workflow.\n- Do NOT output markdown, commentary, or code fences.\n- Use a DAG with parallelism when multiple agents are independent.\n- Use join.all to aggregate parallel branches and then a final synthesizer node.\n- Use depends_on for edges between nodes.\n- Bind node outputs using {{{{placeholders}}}} when passing data forward.\n- Tool contract:\n  - Target tools.v0 client tools (see docs/reference/tools.md).\n  - Workspace access MUST use these exact function tool names:\n    - {tools}\n  - Prefer fs.* tools for reading/listing/searching the workspace (use bash only when necessary).\n  - Do NOT invent ad-hoc tool names (no repo.*, github.*, filesystem.*, etc.).\n  - All client tools MUST be represented as type=\"function\" tools.\n  - Any node that includes tools MUST set tool_execution.mode=\"client\".\n- Prefer minimal nodes needed to satisfy the task.\n",
    )
}

fn plugin_orchestration_system_prompt() -> String {
    "You plan which plugin agents to run based only on their descriptions.\n\nRules:\n- Output MUST be a single JSON object that matches orchestration.plan.v1.\n- Do NOT output markdown, commentary, or code fences.\n- Select only from the provided agent IDs.\n- Prefer minimal agents needed to satisfy the user task.\n- Use multiple steps only when later agents must build on earlier results.\n- Each step can run agents in parallel.\n- Use \"id\" + \"depends_on\" if you need non-sequential step ordering.\n"
        .to_string()
}

fn build_plugin_conversion_prompt(
    plugin: &Plugin,
    command: &PluginCommand,
    user_task: &str,
) -> Result<String, PluginError> {
    let mut out = Vec::new();
    out.push(format!("PLUGIN_URL: {}", plugin.url));
    out.push(format!("COMMAND: {}", command.name));
    out.push("USER_TASK:".to_string());
    out.push(user_task.trim().to_string());
    out.push(String::new());
    out.push("PLUGIN_MANIFEST:".to_string());
    let manifest_json = serde_json::to_string(&plugin.manifest)
        .map_err(|e| PluginError::Loader(format!("failed to serialize plugin manifest: {}", e)))?;
    out.push(manifest_json);
    out.push(String::new());
    out.push(format!("COMMAND_MARKDOWN (commands/{}.md):", command.name));
    out.push(command.prompt.clone());
    out.push(String::new());
    let mut agent_names: Vec<_> = plugin.agents.keys().cloned().collect();
    agent_names.sort();
    if !agent_names.is_empty() {
        out.push("AGENTS_MARKDOWN:".to_string());
        for name in agent_names {
            if let Some(agent) = plugin.agents.get(&name) {
                out.push(format!("---- agents/{}.md ----", name));
                out.push(agent.system_prompt.clone());
                out.push(String::new());
            }
        }
    }
    Ok(out.join("\n"))
}

#[derive(Debug, Clone)]
struct OrchestrationCandidate {
    name: PluginAgentName,
    description: String,
}

fn build_orchestration_candidates(
    plugin: &Plugin,
    command: &PluginCommand,
) -> Result<
    (
        Vec<OrchestrationCandidate>,
        HashMap<PluginAgentName, PluginAgent>,
    ),
    PluginError,
> {
    let names: Vec<PluginAgentName> = if let Some(ref refs) = command.agent_refs {
        if refs.is_empty() {
            plugin.agents.keys().cloned().collect()
        } else {
            refs.clone()
        }
    } else {
        plugin.agents.keys().cloned().collect()
    };
    if names.is_empty() {
        return Err(PluginOrchestrationError::new(
            PluginOrchestrationErrorCode::InvalidPlan,
            "no agents available for dynamic orchestration",
        )
        .into());
    }
    let mut lookup = HashMap::new();
    let mut candidates = Vec::new();
    for name in names {
        let agent = plugin.agents.get(&name).ok_or_else(|| {
            PluginOrchestrationError::new(
                PluginOrchestrationErrorCode::UnknownAgent,
                format!("agent \"{}\" not found", name),
            )
        })?;
        let desc = agent.description.as_ref().map(|s| s.trim()).unwrap_or("");
        if desc.is_empty() {
            return Err(PluginOrchestrationError::new(
                PluginOrchestrationErrorCode::MissingDescription,
                format!("agent \"{}\" missing description", name),
            )
            .into());
        }
        lookup.insert(name.clone(), agent.clone());
        candidates.push(OrchestrationCandidate {
            name,
            description: desc.to_string(),
        });
    }
    Ok((candidates, lookup))
}

fn build_plugin_orchestration_prompt(
    plugin: &Plugin,
    command: &PluginCommand,
    user_task: &str,
    candidates: &[OrchestrationCandidate],
) -> String {
    let mut out = Vec::new();
    if let Some(name) = plugin
        .manifest
        .name
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        out.push(format!("PLUGIN_NAME: {}", name.trim()));
    }
    if let Some(desc) = plugin
        .manifest
        .description
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        out.push(format!("PLUGIN_DESCRIPTION: {}", desc.trim()));
    }
    out.push(format!("COMMAND: {}", command.name));
    out.push("USER_TASK:".to_string());
    out.push(user_task.trim().to_string());
    out.push(String::new());
    if !command.prompt.trim().is_empty() {
        out.push("COMMAND_MARKDOWN:".to_string());
        out.push(command.prompt.trim().to_string());
        out.push(String::new());
    }
    out.push("CANDIDATE_AGENTS:".to_string());
    for candidate in candidates {
        out.push(format!("- id: {}", candidate.name));
        out.push(format!("  description: {}", candidate.description));
    }
    out.join("\n")
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct OrchestrationPlanV1 {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_parallelism: Option<u32>,
    steps: Vec<OrchestrationPlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct OrchestrationPlanStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    depends_on: Option<Vec<String>>,
    agents: Vec<OrchestrationPlanAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct OrchestrationPlanAgent {
    id: String,
    reason: String,
}

fn validate_orchestration_plan(
    plan: &OrchestrationPlanV1,
    lookup: &HashMap<PluginAgentName, PluginAgent>,
) -> Result<(), PluginError> {
    if plan.kind != "orchestration.plan.v1" {
        return Err(PluginOrchestrationError::new(
            PluginOrchestrationErrorCode::InvalidPlan,
            "plan kind must be orchestration.plan.v1",
        )
        .into());
    }
    if let Some(max) = plan.max_parallelism {
        if max < 1 {
            return Err(PluginOrchestrationError::new(
                PluginOrchestrationErrorCode::InvalidPlan,
                "max_parallelism must be >= 1",
            )
            .into());
        }
    }
    let mut step_ids = HashMap::new();
    let mut has_explicit_deps = false;
    for (idx, step) in plan.steps.iter().enumerate() {
        if step
            .depends_on
            .as_ref()
            .map(|d| !d.is_empty())
            .unwrap_or(false)
        {
            has_explicit_deps = true;
        }
        if let Some(id) = step.id.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if step_ids.insert(id.to_string(), idx).is_some() {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::InvalidPlan,
                    format!("duplicate step id \"{}\"", id),
                )
                .into());
            }
        }
    }
    if has_explicit_deps {
        for step in &plan.steps {
            if step.id.as_ref().map(|s| s.trim()).unwrap_or("").is_empty() {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::InvalidPlan,
                    "step id required when depends_on is used",
                )
                .into());
            }
        }
    }

    let mut seen_agents = HashSet::new();
    for (idx, step) in plan.steps.iter().enumerate() {
        if step.agents.is_empty() {
            return Err(PluginOrchestrationError::new(
                PluginOrchestrationErrorCode::InvalidPlan,
                format!("step {} must include at least one agent", idx + 1),
            )
            .into());
        }
        if let Some(deps) = step.depends_on.as_ref() {
            for dep in deps {
                let dep_id = dep.trim();
                if dep_id.is_empty() {
                    return Err(PluginOrchestrationError::new(
                        PluginOrchestrationErrorCode::InvalidDependency,
                        format!("step {} has empty depends_on", idx + 1),
                    )
                    .into());
                }
                let dep_index = step_ids.get(dep_id).copied();
                if dep_index.is_none() {
                    return Err(PluginOrchestrationError::new(
                        PluginOrchestrationErrorCode::InvalidDependency,
                        format!("step {} depends on unknown step \"{}\"", idx + 1, dep_id),
                    )
                    .into());
                }
                if dep_index.unwrap() >= idx {
                    return Err(PluginOrchestrationError::new(
                        PluginOrchestrationErrorCode::InvalidDependency,
                        format!("step {} depends on future step \"{}\"", idx + 1, dep_id),
                    )
                    .into());
                }
            }
        }
        for agent in &step.agents {
            let id = agent.id.trim();
            if id.is_empty() {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::InvalidPlan,
                    format!("step {} agent id required", idx + 1),
                )
                .into());
            }
            let key = PluginAgentName::new(id.to_string())?;
            if !lookup.contains_key(&key) {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::UnknownAgent,
                    format!("unknown agent \"{}\"", id),
                )
                .into());
            }
            if agent.reason.trim().is_empty() {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::InvalidPlan,
                    format!("agent \"{}\" must include a reason", id),
                )
                .into());
            }
            if !seen_agents.insert(id.to_string()) {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::InvalidPlan,
                    format!("agent \"{}\" referenced more than once", id),
                )
                .into());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct StepDependency {
    step_id: String,
    node_id: String,
}

fn build_dynamic_workflow_from_plan(
    plugin: &Plugin,
    command: &PluginCommand,
    task: &str,
    plan: &OrchestrationPlanV1,
    lookup: &HashMap<PluginAgentName, PluginAgent>,
    model: &Model,
) -> Result<WorkflowIntentSpec, PluginError> {
    let step_keys: Vec<String> = plan
        .steps
        .iter()
        .enumerate()
        .map(|(idx, step)| {
            step.id
                .clone()
                .unwrap_or_else(|| format!("step_{}", idx + 1))
        })
        .collect();
    let mut step_order = HashMap::new();
    for (idx, key) in step_keys.iter().enumerate() {
        step_order.insert(key.clone(), idx);
    }
    let mut step_outputs: HashMap<String, String> = HashMap::new();
    let mut used_node_ids: HashSet<String> = HashSet::new();
    let mut nodes: Vec<WorkflowIntentNode> = Vec::new();
    let has_explicit_deps = plan.steps.iter().any(|step| {
        step.depends_on
            .as_ref()
            .map(|d| !d.is_empty())
            .unwrap_or(false)
    });

    for (idx, step) in plan.steps.iter().enumerate() {
        let step_key = &step_keys[idx];
        let dependency_keys: Vec<String> = if has_explicit_deps {
            step.depends_on.clone().unwrap_or_default()
        } else if idx > 0 {
            vec![step_keys[idx - 1].clone()]
        } else {
            Vec::new()
        };
        let deps = dependency_keys
            .iter()
            .map(|raw| {
                let key = raw.trim().to_string();
                let node_id = step_outputs.get(&key).cloned().ok_or_else(|| {
                    PluginOrchestrationError::new(
                        PluginOrchestrationErrorCode::InvalidDependency,
                        format!("missing output for dependency \"{}\"", key),
                    )
                })?;
                let dep_index = step_order.get(&key).copied().ok_or_else(|| {
                    PluginOrchestrationError::new(
                        PluginOrchestrationErrorCode::InvalidDependency,
                        format!("invalid dependency \"{}\"", key),
                    )
                })?;
                if dep_index >= idx {
                    return Err(PluginOrchestrationError::new(
                        PluginOrchestrationErrorCode::InvalidDependency,
                        format!("invalid dependency \"{}\"", key),
                    ));
                }
                Ok(StepDependency {
                    step_id: key,
                    node_id,
                })
            })
            .collect::<Result<Vec<_>, PluginOrchestrationError>>()?;

        let mut step_node_ids = Vec::new();
        for selection in &step.agents {
            let agent_name = selection.id.trim();
            let agent_key = PluginAgentName::new(agent_name.to_string())?;
            let agent = lookup.get(&agent_key).ok_or_else(|| {
                PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::UnknownAgent,
                    format!("unknown agent \"{}\"", agent_name),
                )
            })?;
            let node_id = format_agent_node_id(agent_name)?;
            if !used_node_ids.insert(node_id.clone()) {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::InvalidPlan,
                    format!("duplicate node id \"{}\"", node_id),
                )
                .into());
            }
            let tools = build_tool_refs(agent, command)?;
            let node = WorkflowIntentNode {
                id: node_id.clone(),
                node_type: WorkflowIntentNodeType::Llm,
                depends_on: if deps.is_empty() {
                    None
                } else {
                    Some(deps.iter().map(|d| d.node_id.clone()).collect())
                },
                model: None,
                system: trim_non_empty(&agent.system_prompt),
                user: Some(build_dynamic_agent_user_prompt(command, task, &deps)),
                input: None,
                stream: None,
                tools: if tools.is_empty() {
                    None
                } else {
                    Some(
                        tools
                            .into_iter()
                            .map(|tool| WorkflowIntentToolRef::Name(tool.as_str().to_string()))
                            .collect(),
                    )
                },
                tool_execution: None,
                limit: None,
                timeout_ms: None,
                predicate: None,
                items_from: None,
                items_from_input: None,
                items_pointer: None,
                items_path: None,
                subnode: None,
                max_parallelism: None,
                object: None,
                merge: None,
            };
            nodes.push(node);
            step_node_ids.push(node_id);
        }

        let mut output_node_id = step_node_ids
            .first()
            .cloned()
            .ok_or_else(|| PluginError::Conversion("step produced no node ids".to_string()))?;
        if step_node_ids.len() > 1 {
            let join_id = format_step_join_node_id(step_key)?;
            if !used_node_ids.insert(join_id.clone()) {
                return Err(PluginOrchestrationError::new(
                    PluginOrchestrationErrorCode::InvalidPlan,
                    format!("duplicate node id \"{}\"", join_id),
                )
                .into());
            }
            nodes.push(WorkflowIntentNode {
                id: join_id.clone(),
                node_type: WorkflowIntentNodeType::JoinAll,
                depends_on: Some(step_node_ids.clone()),
                model: None,
                system: None,
                user: None,
                input: None,
                stream: None,
                tools: None,
                tool_execution: None,
                limit: None,
                timeout_ms: None,
                predicate: None,
                items_from: None,
                items_from_input: None,
                items_pointer: None,
                items_path: None,
                subnode: None,
                max_parallelism: None,
                object: None,
                merge: None,
            });
            output_node_id = join_id;
        }
        step_outputs.insert(step_key.clone(), output_node_id);
    }

    let terminal_outputs =
        find_terminal_outputs(&step_keys, plan, &step_outputs, has_explicit_deps);
    let synth_id = "orchestrator_synthesize".to_string();
    let synth_node = WorkflowIntentNode {
        id: synth_id.clone(),
        node_type: WorkflowIntentNodeType::Llm,
        depends_on: Some(terminal_outputs.clone()),
        model: None,
        system: None,
        user: Some(build_dynamic_synthesis_prompt(
            command,
            task,
            &terminal_outputs,
        )),
        input: None,
        stream: None,
        tools: None,
        tool_execution: None,
        limit: None,
        timeout_ms: None,
        predicate: None,
        items_from: None,
        items_from_input: None,
        items_pointer: None,
        items_path: None,
        subnode: None,
        max_parallelism: None,
        object: None,
        merge: None,
    };
    nodes.push(synth_node);

    Ok(WorkflowIntentSpec {
        kind: WorkflowIntentKind::WorkflowIntent,
        name: plugin
            .manifest
            .name
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .or_else(|| Some(command.name.to_string())),
        model: Some(model.as_str().to_string()),
        max_parallelism: plan.max_parallelism.map(|v| v as i64),
        inputs: None,
        nodes,
        outputs: vec![WorkflowIntentOutputRef {
            name: "result".to_string(),
            from: synth_id,
            pointer: None,
        }],
    })
}

fn build_dynamic_agent_user_prompt(
    command: &PluginCommand,
    task: &str,
    deps: &[StepDependency],
) -> String {
    let mut parts = Vec::new();
    if !command.prompt.trim().is_empty() {
        parts.push(command.prompt.trim().to_string());
    }
    parts.push("USER_TASK:".to_string());
    parts.push(task.trim().to_string());
    if !deps.is_empty() {
        parts.push(String::new());
        parts.push("PREVIOUS_STEP_OUTPUTS:".to_string());
        for dep in deps {
            parts.push(format!("- {}: {{{{{}}}}}", dep.step_id, dep.node_id));
        }
    }
    parts.join("\n")
}

fn build_dynamic_synthesis_prompt(
    command: &PluginCommand,
    task: &str,
    outputs: &[String],
) -> String {
    let mut parts = vec!["Synthesize the results and complete the task.".to_string()];
    if !command.prompt.trim().is_empty() {
        parts.push(String::new());
        parts.push("COMMAND:".to_string());
        parts.push(command.prompt.trim().to_string());
    }
    parts.push(String::new());
    parts.push("USER_TASK:".to_string());
    parts.push(task.trim().to_string());
    if !outputs.is_empty() {
        parts.push(String::new());
        parts.push("RESULTS:".to_string());
        for id in outputs {
            parts.push(format!("- {{{{{}}}}}", id));
        }
    }
    parts.join("\n")
}

fn build_tool_refs(
    agent: &PluginAgent,
    command: &PluginCommand,
) -> Result<Vec<PluginToolName>, PluginError> {
    let names = if let Some(tools) = agent.tools.clone().filter(|t| !t.is_empty()) {
        tools
    } else if let Some(tools) = command.tools.clone().filter(|t| !t.is_empty()) {
        tools
    } else {
        DEFAULT_DYNAMIC_TOOLS.to_vec()
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name.as_str().to_string()) {
            continue;
        }
        out.push(name);
    }
    Ok(out)
}

fn find_terminal_outputs(
    step_keys: &[String],
    plan: &OrchestrationPlanV1,
    outputs: &HashMap<String, String>,
    explicit: bool,
) -> Vec<String> {
    if !explicit {
        return step_keys
            .last()
            .and_then(|key| outputs.get(key).cloned())
            .map(|id| vec![id])
            .unwrap_or_default();
    }
    let mut depended = HashSet::new();
    for step in &plan.steps {
        if let Some(deps) = step.depends_on.as_ref() {
            for dep in deps {
                depended.insert(dep.trim().to_string());
            }
        }
    }
    step_keys
        .iter()
        .filter(|key| !depended.contains(*key))
        .filter_map(|key| outputs.get(key).cloned())
        .collect()
}

fn format_agent_node_id(raw: &str) -> Result<String, PluginError> {
    let token = sanitize_node_token(raw);
    if token.is_empty() {
        return Err(PluginOrchestrationError::new(
            PluginOrchestrationErrorCode::InvalidPlan,
            "agent id must contain alphanumeric characters",
        )
        .into());
    }
    Ok(format!("agent_{}", token))
}

fn format_step_join_node_id(step_key: &str) -> Result<String, PluginError> {
    let token = sanitize_node_token(step_key);
    if token.is_empty() {
        return Err(PluginOrchestrationError::new(
            PluginOrchestrationErrorCode::InvalidPlan,
            "step id must contain alphanumeric characters",
        )
        .into());
    }
    if token.starts_with("step_") {
        Ok(format!("{}_join", token))
    } else {
        Ok(format!("step_{}_join", token))
    }
}

fn sanitize_node_token(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

fn workflow_intent_schema() -> Result<OutputFormat, PluginError> {
    let schema_json = include_str!("workflow_v1.schema.json");
    let schema_value: Value = serde_json::from_str(schema_json)
        .map_err(|err| PluginError::Conversion(format!("workflow schema parse: {err}")))?;
    Ok(OutputFormat::json_schema("workflow", schema_value))
}

fn parse_response_json<T: for<'de> Deserialize<'de>>(
    response: crate::types::Response,
) -> Result<T, PluginError> {
    let content = response.text();
    if content.trim().is_empty() {
        return Err(PluginError::Conversion(
            "response contained no content".to_string(),
        ));
    }
    serde_json::from_str(&content)
        .map_err(|err| PluginError::Conversion(format!("failed to parse response JSON: {err}")))
}

fn validate_workflow_tools(spec: &mut WorkflowIntentSpec) -> Result<(), PluginError> {
    for node in &mut spec.nodes {
        if node.node_type != WorkflowIntentNodeType::Llm {
            continue;
        }
        let tools = match node.tools.as_ref() {
            Some(tools) if !tools.is_empty() => tools.clone(),
            _ => continue,
        };
        for tool in &tools {
            let name = match tool {
                WorkflowIntentToolRef::Name(name) => name.trim(),
                WorkflowIntentToolRef::Tool(tool) => {
                    if tool.kind != ToolType::Function {
                        return Err(PluginError::Conversion(
                            "plugin conversion only supports tools.v0 function tools".to_string(),
                        ));
                    }
                    let name = tool
                        .function
                        .as_ref()
                        .map(|f| f.name.as_str())
                        .unwrap_or("");
                    name.trim()
                }
            };
            if name.is_empty() {
                return Err(PluginError::Conversion("tool name required".to_string()));
            }
            if name.parse::<PluginToolName>().is_err() {
                return Err(PluginError::Conversion(format!(
                    "unsupported tool \"{}\" (plugin conversion targets tools.v0)",
                    name
                )));
            }
        }
        if let Some(exec) = &node.tool_execution {
            if exec.mode != WorkflowIntentToolExecutionMode::Client {
                return Err(PluginError::Conversion(
                    "tool_execution.mode must be \"client\" for plugin conversion".to_string(),
                ));
            }
        }
        node.tool_execution = Some(WorkflowIntentToolExecution {
            mode: WorkflowIntentToolExecutionMode::Client,
        });
    }
    Ok(())
}

fn spec_requires_tools(spec: &WorkflowIntentSpec) -> bool {
    fn check_node(node: &WorkflowIntentNode) -> bool {
        if node.node_type == WorkflowIntentNodeType::Llm
            && node.tools.as_ref().map(|t| !t.is_empty()).unwrap_or(false)
        {
            return true;
        }
        if node.node_type == WorkflowIntentNodeType::MapFanout {
            if let Some(sub) = node.subnode.as_ref() {
                return check_node(sub);
            }
        }
        false
    }
    spec.nodes.iter().any(check_node)
}

async fn ensure_model_supports_tools(client: &Client, model: &Model) -> Result<(), PluginError> {
    let model_id = model.as_str().trim();
    if model_id.is_empty() {
        return Err(PluginError::Conversion("model is required".to_string()));
    }
    let response = client.models_with_capability("tools").await?;
    let found = response
        .models
        .iter()
        .any(|entry| entry.model_id.to_string().trim() == model_id);
    if !found {
        return Err(PluginOrchestrationError::new(
            PluginOrchestrationErrorCode::InvalidToolConfig,
            format!("model \"{}\" does not support tool calling", model_id),
        )
        .into());
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PendingToolCallInput {
    id: String,
    name: String,
    arguments: String,
}

async fn execute_pending_tools(
    pending: &[PendingToolCallInput],
    registry: &ToolRegistry,
    handled: &mut HashSet<String>,
) -> Result<Vec<RunsToolResultItemV0>, PluginError> {
    let mut results = Vec::new();
    for call in pending {
        let tool_call_id = call.id.to_string();
        if tool_call_id.trim().is_empty() {
            continue;
        }
        if handled.contains(&tool_call_id) {
            continue;
        }
        let name = call.name.to_string();
        if name.trim().is_empty() {
            continue;
        }
        let tc = ToolCall {
            id: tool_call_id.clone(),
            kind: ToolType::Function,
            function: Some(FunctionCall {
                name: name.clone(),
                arguments: call.arguments.clone(),
            }),
        };
        let exec = registry.execute(&tc).await;
        let output = tool_execution_output(exec);
        let tool_call_id_parsed = ToolCallId::try_from(call.id.clone())
            .map_err(|err| PluginError::Conversion(format!("tool call id is invalid: {err}")))?;
        let tool_name = ToolName::try_from(call.name.clone())
            .map_err(|err| PluginError::Conversion(format!("tool name is invalid: {err}")))?;
        results.push(RunsToolResultItemV0 {
            tool_call: RunsToolCallV0 {
                id: tool_call_id_parsed,
                name: tool_name,
                arguments: None,
            },
            output,
        });
        handled.insert(tool_call_id);
    }
    Ok(results)
}

fn tool_execution_output(res: ToolExecutionResult) -> String {
    if let Some(err) = res.error {
        return format!("Error: {}", err);
    }
    match res.result {
        None => String::new(),
        Some(Value::String(s)) => s,
        Some(val) => serde_json::to_string(&val)
            .unwrap_or_else(|err| format!("Error: failed to marshal tool result: {}", err)),
    }
}

fn trim_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_plugin_manifest(markdown: &str) -> PluginManifest {
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        return PluginManifest::default();
    }
    if let Some(front) = parse_manifest_frontmatter(trimmed) {
        return front;
    }
    let cleaned = trimmed.replace("\r\n", "\n");
    let lines: Vec<&str> = cleaned.lines().collect();
    let mut name = None;
    for line in &lines {
        if let Some(rest) = line.strip_prefix("# ") {
            name = Some(rest.trim().to_string());
            break;
        }
    }
    let mut description = None;
    if name.is_some() {
        let mut after = false;
        for line in &lines {
            let t = line.trim();
            if t.starts_with("# ") {
                after = true;
                continue;
            }
            if !after || t.is_empty() {
                continue;
            }
            if t.starts_with("## ") {
                break;
            }
            description = Some(t.to_string());
            break;
        }
    }
    PluginManifest {
        name,
        description,
        version: None,
        commands: None,
        agents: None,
    }
}

fn parse_manifest_frontmatter(markdown: &str) -> Option<PluginManifest> {
    let lines = split_lines(markdown);
    if lines.is_empty() || lines[0].trim() != "---" {
        return None;
    }
    let end_idx = lines
        .iter()
        .enumerate()
        .find(|(idx, line)| *idx > 0 && line.trim() == "---")
        .map(|(idx, _)| idx)?;
    let mut manifest = PluginManifest::default();
    let mut current_list: Option<&str> = None;
    for line in lines.iter().skip(1).take(end_idx - 1) {
        let raw = line.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        if let Some(list) = current_list {
            if raw.starts_with("- ") {
                let item = raw.trim_start_matches("- ").trim();
                if !item.is_empty() {
                    if list == "commands" {
                        if let Ok(name) = PluginCommandName::new(item.to_string()) {
                            manifest.commands.get_or_insert(Vec::new()).push(name);
                        }
                    } else if list == "agents" {
                        if let Ok(name) = PluginAgentName::new(item.to_string()) {
                            manifest.agents.get_or_insert(Vec::new()).push(name);
                        }
                    }
                }
                continue;
            }
        }
        current_list = None;
        let mut parts = raw.splitn(2, ':');
        let key = parts.next()?.trim().to_lowercase();
        let value = parts.next()?.trim().trim_matches(['"', '\'']).to_string();
        match key.as_str() {
            "name" => manifest.name = Some(value),
            "description" => manifest.description = Some(value),
            "version" => manifest.version = Some(value),
            "commands" => {
                if value.is_empty() {
                    current_list = Some("commands");
                } else {
                    for item in split_frontmatter_list(&value) {
                        if let Ok(name) = PluginCommandName::new(item) {
                            manifest.commands.get_or_insert(Vec::new()).push(name);
                        }
                    }
                }
            }
            "agents" => {
                if value.is_empty() {
                    current_list = Some("agents");
                } else {
                    for item in split_frontmatter_list(&value) {
                        if let Ok(name) = PluginAgentName::new(item) {
                            manifest.agents.get_or_insert(Vec::new()).push(name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(ref mut commands) = manifest.commands {
        commands.sort();
    }
    if let Some(ref mut agents) = manifest.agents {
        agents.sort();
    }
    Some(manifest)
}

struct FrontMatter {
    description: Option<String>,
    tools: Option<Vec<PluginToolName>>,
    body: String,
}

fn parse_markdown_frontmatter(markdown: &str) -> Result<FrontMatter, PluginError> {
    let trimmed = markdown.trim();
    if !trimmed.starts_with("---") {
        return Ok(FrontMatter {
            description: None,
            tools: None,
            body: markdown.to_string(),
        });
    }
    let lines = split_lines(trimmed);
    let end_idx = match lines
        .iter()
        .enumerate()
        .find(|(idx, line)| *idx > 0 && line.trim() == "---")
    {
        Some((idx, _)) => idx,
        None => {
            return Ok(FrontMatter {
                description: None,
                tools: None,
                body: markdown.to_string(),
            })
        }
    };
    let mut description = None;
    let mut tool_items: Vec<String> = Vec::new();
    let mut current_list = None;
    for line in lines.iter().skip(1).take(end_idx - 1) {
        let raw = line.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        if let Some(list) = current_list {
            if list == "tools" && raw.starts_with("- ") {
                let item = raw.trim_start_matches("- ").trim();
                if !item.is_empty() {
                    tool_items.push(item.to_string());
                }
                continue;
            }
        }
        current_list = None;
        let mut parts = raw.splitn(2, ':');
        let key = parts.next().unwrap_or("").trim().to_lowercase();
        let value = parts
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['"', '\''])
            .to_string();
        if key == "description" {
            description = Some(value.clone());
        }
        if key == "tools" {
            if value.is_empty() {
                current_list = Some("tools");
            } else {
                tool_items.extend(split_frontmatter_list(&value));
            }
        }
    }
    let tools = if tool_items.is_empty() {
        None
    } else {
        let mut out = Vec::new();
        for item in tool_items {
            let tool = item
                .parse::<PluginToolName>()
                .map_err(PluginError::Orchestration)?;
            out.push(tool);
        }
        Some(out)
    };
    let body = lines[end_idx + 1..].join("\n");
    let body = body.trim_start_matches(['\n', '\r']).to_string();
    Ok(FrontMatter {
        description,
        tools,
        body,
    })
}

fn split_frontmatter_list(raw: &str) -> Vec<String> {
    let cleaned = raw.trim().trim_start_matches('[').trim_end_matches(']');
    if cleaned.is_empty() {
        return Vec::new();
    }
    cleaned
        .split(',')
        .map(|part| part.trim().trim_matches(['"', '\'']).to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

fn extract_agent_refs(markdown: &str) -> Option<Vec<PluginAgentName>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in split_lines(markdown) {
        let lower = line.to_lowercase();
        let idx = match lower.find("agents/") {
            Some(idx) => idx,
            None => continue,
        };
        if !lower[idx..].contains(".md") {
            continue;
        }
        let mut seg = line[idx..].trim().to_string();
        seg = seg.trim_start_matches("agents/").to_string();
        if let Some(pos) = seg.find(".md") {
            seg = seg[..pos].to_string();
        }
        seg = seg.replace(['`', '*', ' ', '_'], "");
        let seg = seg.trim();
        if seg.is_empty() || seen.contains(seg) {
            continue;
        }
        if let Ok(name) = PluginAgentName::new(seg.to_string()) {
            seen.insert(seg.to_string());
            out.push(name);
        }
    }
    if out.is_empty() {
        None
    } else {
        out.sort();
        Some(out)
    }
}

fn split_lines(input: &str) -> Vec<String> {
    input
        .replace("\r\n", "\n")
        .lines()
        .map(|s| s.to_string())
        .collect()
}

fn basename(path: &str) -> String {
    path.split('/')
        .next_back()
        .unwrap_or("")
        .trim_end_matches(".md")
        .to_string()
}

fn join_repo_path(base: &str, elem: &str) -> String {
    let clean = |value: &str| value.trim_matches('/').to_string();
    let b = clean(base);
    let e = clean(elem);
    if b.is_empty() {
        return e;
    }
    if e.is_empty() {
        return b;
    }
    format!("{}/{}", b, e)
}

fn sorted_keys<T: Ord>(mut items: Vec<T>) -> Vec<T> {
    items.sort();
    items
}

fn clone_plugin(plugin: &Plugin) -> Plugin {
    Plugin {
        id: plugin.id.clone(),
        url: plugin.url.clone(),
        manifest: plugin.manifest.clone(),
        commands: plugin.commands.clone(),
        agents: plugin.agents.clone(),
        raw_files: plugin.raw_files.clone(),
        git_ref: plugin.git_ref.clone(),
        loaded_at: plugin.loaded_at,
    }
}

fn derive_plugin_id(owner: &str, repo: &str, repo_path: &str) -> String {
    let base = format!("{}/{}", owner.trim(), repo.trim());
    if repo_path.trim().is_empty() {
        base
    } else {
        format!("{}/{}", base, repo_path.trim_matches('/'))
    }
}

#[derive(Debug)]
struct PluginHTTPError {
    status: StatusCode,
    message: String,
}

impl PluginHTTPError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn is_not_found(&self) -> bool {
        self.status == StatusCode::NOT_FOUND
    }
}

impl fmt::Display for PluginHTTPError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.trim().is_empty() {
            write!(f, "http error ({})", self.status)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

fn parse_github_plugin_ref(raw: &str) -> Result<GitHubPluginRef, PluginError> {
    let mut url = raw.trim().to_string();
    if url.is_empty() {
        return Err(PluginError::Loader("source url required".to_string()));
    }
    if url.starts_with("git@github.com:") {
        url = format!("https://github.com/{}", url.replace("git@github.com:", ""));
    }
    if !url.contains("://") {
        url = format!("https://{}", url);
    }
    let parsed = Url::parse(&url)
        .map_err(|err| PluginError::Loader(format!("invalid github url: {err}")))?;
    let host = parsed.host_str().unwrap_or("").trim_start_matches("www.");
    if host != "github.com" && host != "raw.githubusercontent.com" {
        return Err(PluginError::Loader(format!(
            "unsupported host: {}",
            parsed.host_str().unwrap_or("")
        )));
    }
    let mut ref_name = parsed
        .query_pairs()
        .find(|(k, _)| k == "ref")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    let parts: Vec<&str> = parsed
        .path()
        .split('/')
        .filter(|seg| !seg.is_empty())
        .collect();
    if parts.len() < 2 {
        return Err(PluginError::Loader(
            "invalid github url: expected /owner/repo".to_string(),
        ));
    }
    let owner = parts[0].to_string();
    let mut repo_part = parts[1].trim_end_matches(".git").to_string();
    if let Some(at_idx) = repo_part.find('@') {
        if ref_name.is_empty() && at_idx + 1 < repo_part.len() {
            ref_name = repo_part[at_idx + 1..].to_string();
        }
        repo_part = repo_part[..at_idx].to_string();
    }
    let repo = repo_part;
    let mut rest: Vec<&str> = parts.into_iter().skip(2).collect();
    if host == "github.com" && rest.len() >= 2 && (rest[0] == "tree" || rest[0] == "blob") {
        if ref_name.is_empty() {
            ref_name = rest[1].to_string();
        }
        rest = rest.into_iter().skip(2).collect();
    }
    if host == "raw.githubusercontent.com" {
        if rest.is_empty() {
            return Err(PluginError::Loader("invalid raw github url".to_string()));
        }
        if ref_name.is_empty() {
            ref_name = rest[0].to_string();
        }
        rest = rest.into_iter().skip(1).collect();
    }
    let mut repo_path = rest.join("/");
    repo_path = repo_path.trim_matches('/').to_string();
    if repo_path.ends_with("PLUGIN.md") || repo_path.ends_with("SKILL.md") {
        repo_path = repo_path
            .split('/')
            .take(repo_path.split('/').count().saturating_sub(1))
            .collect::<Vec<_>>()
            .join("/");
    }
    if repo_path.ends_with(".md") {
        if let Some(idx) = repo_path.find("/commands/") {
            repo_path = repo_path[..idx].to_string();
        }
        if let Some(idx) = repo_path.find("/agents/") {
            repo_path = repo_path[..idx].to_string();
        }
        repo_path = repo_path.trim_matches('/').to_string();
    }
    if ref_name.is_empty() {
        ref_name = DEFAULT_PLUGIN_REF.to_string();
    }
    let canonical = if repo_path.is_empty() {
        format!("github.com/{}/{}@{}", owner, repo, ref_name)
    } else {
        format!("github.com/{}/{}@{}/{}", owner, repo, ref_name, repo_path)
    };
    Ok(GitHubPluginRef {
        owner,
        repo,
        git_ref: ref_name,
        repo_path,
        canonical,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiKey, Client, Config, RetryConfig, WorkflowIntentToolExecutionMode};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    #[derive(Clone)]
    struct CaptureResponder {
        store: Arc<Mutex<Option<Value>>>,
        template: ResponseTemplate,
    }

    impl Respond for CaptureResponder {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            if let Ok(value) = serde_json::from_slice::<Value>(&req.body) {
                *self.store.lock().expect("mutex") = Some(value);
            }
            self.template.clone()
        }
    }

    fn client_for_server(server: &MockServer) -> Client {
        Client::new(Config {
            api_key: Some(ApiKey::parse("mr_sk_test_key").expect("api key")),
            base_url: Some(server.uri()),
            retry: Some(RetryConfig {
                max_attempts: 1,
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("client creation should succeed")
    }

    #[test]
    fn parse_markdown_frontmatter_tools() {
        let markdown = r#"---
description: Example agent
tools:
  - fs.read_file
  - bash
---
Agent prompt
"#;
        let parsed = parse_markdown_frontmatter(markdown).expect("parsed");
        assert_eq!(parsed.description.unwrap(), "Example agent");
        let tools = parsed.tools.unwrap();
        assert_eq!(
            tools,
            vec![PluginToolName::FsReadFile, PluginToolName::Bash]
        );
        assert_eq!(parsed.body.trim(), "Agent prompt");
    }

    #[tokio::test]
    async fn to_workflow_dynamic_builds_workflow_from_plan() {
        let server = MockServer::start().await;
        let captured = Arc::new(Mutex::new(None));
        let plan = OrchestrationPlanV1 {
            kind: "orchestration.plan.v1".to_string(),
            max_parallelism: None,
            steps: vec![OrchestrationPlanStep {
                id: None,
                depends_on: None,
                agents: vec![
                    OrchestrationPlanAgent {
                        id: "reviewer".to_string(),
                        reason: "Find bugs".to_string(),
                    },
                    OrchestrationPlanAgent {
                        id: "tester".to_string(),
                        reason: "Check tests".to_string(),
                    },
                ],
            }],
        };
        let raw_plan = serde_json::to_string(&plan).expect("plan json");

        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(CaptureResponder {
                store: captured.clone(),
                template: ResponseTemplate::new(200).set_body_json(json!({
                    "id": "resp_1",
                    "model": "claude-3-5-haiku-latest",
                    "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 },
                    "output": [{
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "text", "text": raw_plan }]
                    }]
                })),
            })
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("capability", "tools"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [{
                    "model_id": "claude-3-5-haiku-latest",
                    "provider": "anthropic",
                    "display_name": "Claude",
                    "description": "",
                    "context_window": 1,
                    "max_output_tokens": 1,
                    "deprecated": false,
                    "deprecation_message": "",
                    "input_cost_per_million_cents": 1,
                    "output_cost_per_million_cents": 1,
                    "training_cutoff": "2025-01",
                    "capabilities": ["tools"]
                }]
            })))
            .mount(&server)
            .await;

        let plugin = Plugin {
            id: PluginId::new("octo/repo/plugins/my").unwrap(),
            url: PluginUrl::new("github.com/octo/repo@main/plugins/my").unwrap(),
            manifest: PluginManifest {
                name: Some("x".to_string()),
                description: None,
                version: None,
                commands: None,
                agents: None,
            },
            commands: {
                let mut map = HashMap::new();
                map.insert(
                    PluginCommandName::new("analyze").unwrap(),
                    PluginCommand {
                        name: PluginCommandName::new("analyze").unwrap(),
                        prompt: "# analyze".to_string(),
                        agent_refs: Some(vec![
                            PluginAgentName::new("reviewer").unwrap(),
                            PluginAgentName::new("tester").unwrap(),
                        ]),
                        tools: None,
                    },
                );
                map
            },
            agents: {
                let mut map = HashMap::new();
                map.insert(
                    PluginAgentName::new("reviewer").unwrap(),
                    PluginAgent {
                        name: PluginAgentName::new("reviewer").unwrap(),
                        system_prompt: "You review code.".to_string(),
                        description: Some("Expert code reviewer.".to_string()),
                        tools: None,
                    },
                );
                map.insert(
                    PluginAgentName::new("tester").unwrap(),
                    PluginAgent {
                        name: PluginAgentName::new("tester").unwrap(),
                        system_prompt: "You run tests.".to_string(),
                        description: Some("Expert test runner.".to_string()),
                        tools: None,
                    },
                );
                map
            },
            raw_files: HashMap::new(),
            git_ref: PluginGitHubRef {
                owner: "octo".to_string(),
                repo: "repo".to_string(),
                git_ref: "main".to_string(),
                path: Some("plugins/my".to_string()),
            },
            loaded_at: Utc::now(),
        };

        let client = client_for_server(&server);
        let converter = PluginConverter::new(client, PluginConverterOptions::default());
        let spec = converter
            .to_workflow_dynamic(&plugin, "analyze", "do the thing")
            .await
            .expect("dynamic spec");

        assert_eq!(spec.kind, WorkflowIntentKind::WorkflowIntent);
        assert_eq!(spec.model.as_deref(), Some("claude-3-5-haiku-latest"));

        let payload = captured
            .lock()
            .expect("mutex")
            .clone()
            .expect("captured request");
        assert_eq!(
            payload["output_format"]["json_schema"]["name"].as_str(),
            Some("orchestration_plan")
        );
        let user_prompt = payload["input"][1]["content"][0]["text"]
            .as_str()
            .unwrap_or("");
        assert!(user_prompt.contains("Expert code reviewer."));
        assert!(user_prompt.contains("Expert test runner."));

        let mut lookup = HashMap::new();
        for node in &spec.nodes {
            lookup.insert(node.id.clone(), node.clone());
        }

        let reviewer = lookup.get("agent_reviewer").expect("reviewer node");
        let tools = reviewer.tools.clone().unwrap_or_default();
        let tool_names: HashSet<String> = tools
            .into_iter()
            .filter_map(|tool| match tool {
                WorkflowIntentToolRef::Name(name) => Some(name),
                WorkflowIntentToolRef::Tool(inner) => inner
                    .function
                    .as_ref()
                    .map(|func| func.name.as_str().to_string()),
            })
            .collect();
        for expected in ["fs.read_file", "fs.list_files", "fs.search"] {
            assert!(tool_names.contains(expected));
        }
        assert!(!tool_names.contains("bash"));
        assert!(!tool_names.contains("write_file"));
        assert_eq!(
            reviewer
                .tool_execution
                .as_ref()
                .expect("tool execution")
                .mode,
            WorkflowIntentToolExecutionMode::Client
        );

        assert!(lookup.contains_key("agent_tester"));
        assert!(lookup.contains_key("step_1_join"));
        let synth = lookup.get("orchestrator_synthesize").expect("synth node");
        assert_eq!(
            synth.depends_on.clone().unwrap_or_default(),
            vec!["step_1_join".to_string()]
        );
        assert_eq!(spec.outputs.len(), 1);
        assert_eq!(spec.outputs[0].from, "orchestrator_synthesize");
    }

    #[tokio::test]
    async fn to_workflow_dynamic_requires_agent_descriptions() {
        let client = Client::new(Config {
            api_key: Some(ApiKey::parse("mr_sk_test_key").expect("api key")),
            base_url: Some("http://example.invalid".to_string()),
            ..Default::default()
        })
        .expect("client creation should succeed");
        let converter = PluginConverter::new(client, PluginConverterOptions::default());

        let plugin = Plugin {
            id: PluginId::new("octo/repo/plugins/my").unwrap(),
            url: PluginUrl::new("github.com/octo/repo@main/plugins/my").unwrap(),
            manifest: PluginManifest {
                name: Some("x".to_string()),
                description: None,
                version: None,
                commands: None,
                agents: None,
            },
            commands: {
                let mut map = HashMap::new();
                map.insert(
                    PluginCommandName::new("analyze").unwrap(),
                    PluginCommand {
                        name: PluginCommandName::new("analyze").unwrap(),
                        prompt: "# analyze".to_string(),
                        agent_refs: None,
                        tools: None,
                    },
                );
                map
            },
            agents: {
                let mut map = HashMap::new();
                map.insert(
                    PluginAgentName::new("reviewer").unwrap(),
                    PluginAgent {
                        name: PluginAgentName::new("reviewer").unwrap(),
                        system_prompt: "You review code.".to_string(),
                        description: Some(String::new()),
                        tools: None,
                    },
                );
                map
            },
            raw_files: HashMap::new(),
            git_ref: PluginGitHubRef {
                owner: "octo".to_string(),
                repo: "repo".to_string(),
                git_ref: "main".to_string(),
                path: Some("plugins/my".to_string()),
            },
            loaded_at: Utc::now(),
        };

        let err = converter
            .to_workflow_dynamic(&plugin, "analyze", "do the thing")
            .await
            .expect_err("expected error");
        assert!(err.to_string().contains("missing description"));
    }

    #[tokio::test]
    async fn to_workflow_dynamic_rejects_unknown_plan_agents() {
        let server = MockServer::start().await;
        let plan = OrchestrationPlanV1 {
            kind: "orchestration.plan.v1".to_string(),
            max_parallelism: None,
            steps: vec![OrchestrationPlanStep {
                id: None,
                depends_on: None,
                agents: vec![OrchestrationPlanAgent {
                    id: "tester".to_string(),
                    reason: "Not allowed".to_string(),
                }],
            }],
        };
        let raw_plan = serde_json::to_string(&plan).expect("plan json");

        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_1",
                "model": "claude-3-5-haiku-latest",
                "usage": { "input_tokens": 1, "output_tokens": 1, "total_tokens": 2 },
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": raw_plan }]
                }]
            })))
            .mount(&server)
            .await;

        let plugin = Plugin {
            id: PluginId::new("octo/repo/plugins/my").unwrap(),
            url: PluginUrl::new("github.com/octo/repo@main/plugins/my").unwrap(),
            manifest: PluginManifest {
                name: Some("x".to_string()),
                description: None,
                version: None,
                commands: None,
                agents: None,
            },
            commands: {
                let mut map = HashMap::new();
                map.insert(
                    PluginCommandName::new("analyze").unwrap(),
                    PluginCommand {
                        name: PluginCommandName::new("analyze").unwrap(),
                        prompt: "# analyze".to_string(),
                        agent_refs: Some(vec![PluginAgentName::new("reviewer").unwrap()]),
                        tools: None,
                    },
                );
                map
            },
            agents: {
                let mut map = HashMap::new();
                map.insert(
                    PluginAgentName::new("reviewer").unwrap(),
                    PluginAgent {
                        name: PluginAgentName::new("reviewer").unwrap(),
                        system_prompt: "You review code.".to_string(),
                        description: Some("Expert code reviewer.".to_string()),
                        tools: None,
                    },
                );
                map.insert(
                    PluginAgentName::new("tester").unwrap(),
                    PluginAgent {
                        name: PluginAgentName::new("tester").unwrap(),
                        system_prompt: "You run tests.".to_string(),
                        description: Some("Expert test runner.".to_string()),
                        tools: None,
                    },
                );
                map
            },
            raw_files: HashMap::new(),
            git_ref: PluginGitHubRef {
                owner: "octo".to_string(),
                repo: "repo".to_string(),
                git_ref: "main".to_string(),
                path: Some("plugins/my".to_string()),
            },
            loaded_at: Utc::now(),
        };

        let client = client_for_server(&server);
        let converter = PluginConverter::new(client, PluginConverterOptions::default());
        let err = converter
            .to_workflow_dynamic(&plugin, "analyze", "do the thing")
            .await
            .expect_err("expected error");
        assert!(err.to_string().contains("unknown agent"));
    }

    #[test]
    fn dynamic_workflow_tool_scoping() {
        let plugin = Plugin {
            id: PluginId::new("acme/example").unwrap(),
            url: PluginUrl::new("github.com/acme/example@HEAD").unwrap(),
            manifest: PluginManifest {
                name: Some("Example".to_string()),
                description: None,
                version: None,
                commands: None,
                agents: None,
            },
            commands: {
                let mut map = HashMap::new();
                map.insert(
                    PluginCommandName::new("run").unwrap(),
                    PluginCommand {
                        name: PluginCommandName::new("run").unwrap(),
                        prompt: "Do it.".to_string(),
                        agent_refs: None,
                        tools: Some(vec![PluginToolName::FsSearch]),
                    },
                );
                map
            },
            agents: {
                let mut map = HashMap::new();
                map.insert(
                    PluginAgentName::new("writer").unwrap(),
                    PluginAgent {
                        name: PluginAgentName::new("writer").unwrap(),
                        system_prompt: "Write".to_string(),
                        description: Some("Writes".to_string()),
                        tools: Some(vec![PluginToolName::FsReadFile]),
                    },
                );
                map.insert(
                    PluginAgentName::new("reviewer").unwrap(),
                    PluginAgent {
                        name: PluginAgentName::new("reviewer").unwrap(),
                        system_prompt: "Review".to_string(),
                        description: Some("Reviews".to_string()),
                        tools: None,
                    },
                );
                map
            },
            raw_files: HashMap::new(),
            git_ref: PluginGitHubRef {
                owner: "acme".to_string(),
                repo: "example".to_string(),
                git_ref: "HEAD".to_string(),
                path: None,
            },
            loaded_at: Utc::now(),
        };
        let plan = OrchestrationPlanV1 {
            kind: "orchestration.plan.v1".to_string(),
            max_parallelism: None,
            steps: vec![
                OrchestrationPlanStep {
                    id: Some("draft".to_string()),
                    depends_on: None,
                    agents: vec![OrchestrationPlanAgent {
                        id: "writer".to_string(),
                        reason: "draft".to_string(),
                    }],
                },
                OrchestrationPlanStep {
                    id: Some("review".to_string()),
                    depends_on: Some(vec!["draft".to_string()]),
                    agents: vec![OrchestrationPlanAgent {
                        id: "reviewer".to_string(),
                        reason: "review".to_string(),
                    }],
                },
            ],
        };
        let command = plugin
            .commands
            .get(&PluginCommandName::new("run").unwrap())
            .unwrap()
            .clone();
        let mut lookup = HashMap::new();
        for (k, v) in &plugin.agents {
            lookup.insert(k.clone(), v.clone());
        }
        let spec = build_dynamic_workflow_from_plan(
            &plugin,
            &command,
            "Ship it",
            &plan,
            &lookup,
            &Model::from("claude-3-5-haiku-latest"),
        )
        .unwrap();
        let writer = spec.nodes.iter().find(|n| n.id == "agent_writer").unwrap();
        let reviewer = spec
            .nodes
            .iter()
            .find(|n| n.id == "agent_reviewer")
            .unwrap();
        assert_eq!(
            writer.tools.clone().unwrap(),
            vec![WorkflowIntentToolRef::Name("fs.read_file".to_string())]
        );
        assert_eq!(
            reviewer.tools.clone().unwrap(),
            vec![WorkflowIntentToolRef::Name("fs.search".to_string())]
        );
        assert_eq!(reviewer.depends_on.clone().unwrap(), vec!["agent_writer"]);
    }
}
