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
}

impl WorkflowKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowKind::WorkflowV0 => "workflow.v0",
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

/// A node in the workflow DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeV0 {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeTypeV0,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

/// An edge connecting two nodes in the workflow DAG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeV0 {
    pub from: NodeId,
    pub to: NodeId,
}

/// Reference to a workflow output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputRefV0 {
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
