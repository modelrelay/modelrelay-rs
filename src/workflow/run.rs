//! Workflow run status and result types.
//!
//! This module contains types for tracking workflow execution:
//! - `RunStatusV0` - Status of a workflow run
//! - `NodeStatusV0` - Status of a node within a run
//! - `NodeResultV0` - Result of a completed node
//! - `RunCostSummaryV0` - Cost summary for a run
//! - `RunCostLineItemV0` - Individual cost line item
//! - `PayloadInfoV0` - Metadata about a payload
//! - `NodeErrorV0` - Error information for a failed node

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use super::ids::{ModelId, NodeId, Sha256Hash};
use super::spec::NodeTypeV0;
use crate::identifiers::ProviderId;

/// Status of a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatusV0 {
    Running,
    Waiting,
    Succeeded,
    Failed,
    Canceled,
}

/// Status of a node within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatusV0 {
    Pending,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Canceled,
}

/// Metadata about a payload (node output or run outputs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PayloadInfoV0 {
    pub bytes: i64,
    pub sha256: Sha256Hash,
    pub included: bool,
}

/// Error information for a failed node or run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeErrorV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

/// Result of a completed node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeResultV0 {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeTypeV0,
    pub status: NodeStatusV0,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<NodeErrorV0>,
}

/// Cost summary for a workflow run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunCostSummaryV0 {
    pub total_usd_cents: i64,
    #[serde(default)]
    pub line_items: Vec<RunCostLineItemV0>,
}

/// Individual cost line item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunCostLineItemV0 {
    pub provider_id: ProviderId,
    pub model: ModelId,
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub usd_cents: i64,
}
