//! Workflow specification types.
//!
//! This module contains the types that define a workflow's structure:
//! - `WorkflowKind` - Version discriminator for workflow specs
//! - `WorkflowSpecV0` - The main workflow specification
//! - `NodeV0` - A node in the workflow DAG
//! - `EdgeV0` - An edge connecting nodes
//! - `ExecutionV0` - Execution configuration
//! - `OutputRefV0` - Reference to a workflow output
//! - `NodeTypeV0` - The type of a node

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::NodeId;

/// Workflow specification version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowKind {
    #[serde(rename = "workflow.v0")]
    WorkflowV0,
    #[serde(rename = "workflow.v1")]
    WorkflowV1,
}

impl WorkflowKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowKind::WorkflowV0 => "workflow.v0",
            WorkflowKind::WorkflowV1 => "workflow.v1",
        }
    }
}

/// Node type within a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeTypeV0 {
    #[serde(rename = "llm.responses")]
    LlmResponses,
    #[serde(rename = "join.all")]
    JoinAll,
    #[serde(rename = "transform.json")]
    TransformJson,
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

/// Workflow specification (v0).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSpecV0 {
    pub kind: WorkflowKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionV0>,
    pub nodes: Vec<NodeV0>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<EdgeV0>>,
    pub outputs: Vec<OutputRefV0>,
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
    pub nodes: Vec<NodeV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<EdgeV1>>,
    pub outputs: Vec<OutputRefV1>,
}

/// Execution configuration for a workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_timeout_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_timeout_ms: Option<i64>,
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

/// A node in the workflow DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeV0 {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeTypeV0,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
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

/// An edge connecting two nodes in the workflow DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeV0 {
    pub from: NodeId,
    pub to: NodeId,
}

/// An edge connecting two nodes in the workflow.v1 DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeV1 {
    pub from: NodeId,
    pub to: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<ConditionV1>,
}

/// Reference to a workflow output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputRefV0 {
    pub name: String,
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
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
//   use modelrelay::workflow::{Kind, SpecV0};
//
// This matches the pattern used in the Go and TypeScript SDKs.

/// Type alias for `WorkflowKind` - allows `workflow::Kind` usage.
#[allow(dead_code)]
pub type Kind = WorkflowKind;

/// Type alias for `WorkflowSpecV0` - allows `workflow::SpecV0` usage.
#[allow(dead_code)]
pub type SpecV0 = WorkflowSpecV0;

/// Type alias for `WorkflowSpecV1` - allows `workflow::SpecV1` usage.
#[allow(dead_code)]
pub type SpecV1 = WorkflowSpecV1;
