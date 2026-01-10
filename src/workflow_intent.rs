use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{InputItem, Tool};

/// Workflow specification kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowIntentKind {
    #[serde(rename = "workflow")]
    WorkflowIntent,
}

impl WorkflowIntentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowIntentKind::WorkflowIntent => "workflow",
        }
    }
}

/// Node type within a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkflowIntentNodeType {
    #[serde(rename = "llm")]
    Llm,
    #[serde(rename = "join.all")]
    JoinAll,
    #[serde(rename = "join.any")]
    JoinAny,
    #[serde(rename = "join.collect")]
    JoinCollect,
    #[serde(rename = "transform.json")]
    TransformJson,
    #[serde(rename = "map.fanout")]
    MapFanout,
}

/// Tool execution mode for LLM nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowIntentToolExecutionMode {
    Server,
    Client,
    Agentic,
}

/// Tool execution configuration for LLM nodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowIntentToolExecution {
    pub mode: WorkflowIntentToolExecutionMode,
}

/// Tool reference for workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum WorkflowIntentToolRef {
    Name(String),
    Tool(Tool),
}

impl WorkflowIntentToolRef {
    pub fn name(&self) -> Option<&str> {
        match self {
            WorkflowIntentToolRef::Name(name) => Some(name.as_str()),
            WorkflowIntentToolRef::Tool(tool) => tool.function.as_ref().map(|f| f.name.as_str()),
        }
    }
}

impl From<&str> for WorkflowIntentToolRef {
    fn from(value: &str) -> Self {
        WorkflowIntentToolRef::Name(value.trim().to_string())
    }
}

impl From<String> for WorkflowIntentToolRef {
    fn from(value: String) -> Self {
        WorkflowIntentToolRef::Name(value.trim().to_string())
    }
}

impl From<Tool> for WorkflowIntentToolRef {
    fn from(value: Tool) -> Self {
        WorkflowIntentToolRef::Tool(value)
    }
}

/// Condition used by join.any/join.collect predicates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowIntentCondition {
    pub source: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// Transform value reference for transform.json nodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowIntentTransformValue {
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

/// A node in the workflow DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowIntentNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: WorkflowIntentNodeType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,

    // LLM node fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<InputItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<WorkflowIntentToolRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_execution: Option<WorkflowIntentToolExecution>,

    // join.collect fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<WorkflowIntentCondition>,

    // map.fanout fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_from_input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subnode: Option<Box<WorkflowIntentNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<i64>,

    // transform.json fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<BTreeMap<String, WorkflowIntentTransformValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<Vec<WorkflowIntentTransformValue>>,
}

impl WorkflowIntentNode {
    /// Creates a new node with the given id and type, all other fields set to None.
    pub fn with_type(id: impl Into<String>, node_type: WorkflowIntentNodeType) -> Self {
        Self {
            id: id.into(),
            node_type,
            depends_on: None,
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
        }
    }
}

/// Reference to a workflow output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowIntentOutputRef {
    pub name: String,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

/// Workflow specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowIntentSpec {
    pub kind: WorkflowIntentKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub nodes: Vec<WorkflowIntentNode>,
    pub outputs: Vec<WorkflowIntentOutputRef>,
}

// Clean type aliases (no Workflow prefix)
#[allow(dead_code)]
pub type IntentKind = WorkflowIntentKind;
#[allow(dead_code)]
pub type IntentSpec = WorkflowIntentSpec;
