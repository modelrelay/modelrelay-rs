//! Workflow run status and result types.
//!
//! This module contains types for tracking workflow execution:
//! - `RunStatusV0` - Status of a workflow run (from generated)
//! - `NodeStatusV0` - Status of a node within a run (from generated)
//! - `NodeErrorV0` - Error information for a failed node (from generated)
//! - `RunCostSummaryV0` - Cost summary for a run (from generated)
//! - `RunCostLineItemV0` - Individual cost line item (from generated)
//! - `NodeResultV0` - Result of a completed node (hand-written for field naming)
//! - `PayloadInfoV0` - Metadata about a payload

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::{ArtifactKey, NodeId, Sha256Hash};
use super::spec::NodeTypeV1;

// Re-export types from generated module (single source of truth)
pub use crate::generated::{
    NodeErrorV0, NodeStatusV0, RunCostLineItemV0, RunCostSummaryV0, RunStatusV0,
};

/// Metadata about a payload (node output or run outputs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PayloadInfoV0 {
    pub bytes: u64,
    pub sha256: Sha256Hash,
    pub included: bool,
}

/// Artifact reference plus payload metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PayloadArtifactV0 {
    pub artifact_key: ArtifactKey,
    pub info: PayloadInfoV0,
}

// NodeErrorV0 is now imported from crate::generated (identical structure)

/// Result of a completed node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeResultV0 {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeTypeV1,
    pub status: NodeStatusV0,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<NodeErrorV0>,
}

// RunCostSummaryV0 and RunCostLineItemV0 are now imported from crate::generated
// (they use ProviderId and ModelId strong types from the OpenAPI spec)
