//! Workflow specification types.
//!
//! This module contains the types that define a workflow's structure:
//! - `WorkflowKind` - Version discriminator for workflow specs
//! - `WorkflowSpecV1` - The main workflow specification
//! - `NodeV1` - A node in the workflow DAG
//! - `EdgeV1` - An edge connecting nodes
//! - `ExecutionV1` - Execution configuration
//! - `OutputRefV1` - Reference to a workflow output
//! - `NodeTypeV1` - The type of a node

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::NodeId;

/// Workflow specification version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowKind {
    #[serde(rename = "workflow.v1")]
    WorkflowV1,
}

/// Node type within a workflow.v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeTypeV1 {
    #[serde(rename = "llm.responses")]
    LlmResponses,
    #[serde(rename = "route.switch")]
    RouteSwitch,
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

/// Condition source for workflow.v1 edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionSourceV1 {
    NodeOutput,
    NodeStatus,
}

/// Condition operator for workflow.v1 edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConditionOpV1 {
    Equals,
    Matches,
    Exists,
}

/// Edge condition for workflow.v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConditionV1 {
    pub source: ConditionSourceV1,
    pub op: ConditionOpV1,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// Workflow specification (v1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSpecV1 {
    pub kind: WorkflowKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<InputDeclV1>>,
    pub nodes: Vec<NodeV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<EdgeV1>>,
    pub outputs: Vec<OutputRefV1>,
}

/// Workflow input declaration (v1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputDeclV1 {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

/// Execution configuration for a workflow.v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_timeout_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_timeout_ms: Option<i64>,
}

/// A node in the workflow.v1 DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeV1 {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeTypeV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

/// An edge connecting two nodes in the workflow.v1 DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeV1 {
    pub from: NodeId,
    pub to: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<ConditionV1>,
}

/// Reference to a workflow.v1 output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputRefV1 {
    pub name: String,
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

// ============================================================================
// Clean type aliases (no Workflow prefix)
// ============================================================================
// These aliases allow users to import from the workflow module with cleaner names:
//   use modelrelay::workflow::{Kind, SpecV1};
//
// This matches the pattern used in the Go and TypeScript SDKs.

/// Type alias for `WorkflowKind` - allows `workflow::Kind` usage.
#[allow(dead_code)]
pub type Kind = WorkflowKind;

/// Type alias for `WorkflowSpecV1` - allows `workflow::SpecV1` usage.
#[allow(dead_code)]
pub type SpecV1 = WorkflowSpecV1;
