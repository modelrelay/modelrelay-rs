//! Workflow types for the ModelRelay workflow engine.
//!
//! This module contains all types related to workflow definitions and execution:
//!
//! - **[`ids`]** - Identifier types (RunId, NodeId, RequestId, etc.)
//! - **[`spec`]** - Workflow specification types (SpecV0, NodeV0, etc.)
//! - **[`run`]** - Run status and result types (RunStatusV0, NodeResultV0, etc.)
//! - **[`events`]** - Event types for run streaming (RunEventV0, etc.)
//!
//! ## Clean type aliases
//!
//! This module exports clean type aliases without the `Workflow` prefix:
//! - `workflow::Kind` (alias for `WorkflowKind`)
//! - `workflow::SpecV0` (alias for `WorkflowSpecV0`)
//!
//! This matches the pattern used in the Go and TypeScript SDKs.
//!
//! ## Example
//!
//! ```ignore
//! use modelrelay::workflow::{Kind, SpecV0, RunId, NodeId, RunEventV0};
//!
//! // Create identifiers
//! let run_id = RunId::new();
//! let node_id: NodeId = "my_node".into();
//!
//! // Use clean type names
//! let spec = SpecV0 {
//!     kind: Kind::WorkflowV0,
//!     // ...
//! };
//!
//! // Parse a run event
//! let event: RunEventV0 = serde_json::from_str(json_str)?;
//! println!("Run {} seq {}", event.run_id(), event.seq());
//! ```

pub mod events;
pub mod ids;
pub mod run;
pub mod spec;

// ============================================================================
// Schema JSON
// ============================================================================

/// JSON Schema for workflow.v0 specifications.
pub const WORKFLOW_V0_SCHEMA_JSON: &str = include_str!("../workflow_v0.schema.json");

/// JSON Schema for run_event.v0 events.
pub const RUN_EVENT_V0_SCHEMA_JSON: &str = include_str!("../run_event_v0.schema.json");

// ============================================================================
// Re-exports from identifiers
// ============================================================================

/// Re-export ProviderId from identifiers module (single source of truth).
pub use crate::identifiers::ProviderId;

// ============================================================================
// Re-exports from ids
// ============================================================================

pub use ids::{ArtifactKey, ModelId, NodeId, PlanHash, RequestId, RunId, Sha256Hash};

// ============================================================================
// Re-exports from spec
// ============================================================================

pub use spec::{
    EdgeV0, ExecutionV0, NodeTypeV0, NodeV0, OutputRefV0, WorkflowKind, WorkflowSpecV0,
};

// Clean type aliases (no Workflow prefix) - matches Go and TypeScript SDK patterns
#[allow(unused_imports)]
pub use spec::{Kind, SpecV0};

// ============================================================================
// Re-exports from run
// ============================================================================

pub use run::{
    NodeErrorV0, NodeResultV0, NodeStatusV0, PayloadInfoV0, RunCostLineItemV0, RunCostSummaryV0,
    RunStatusV0,
};

// ============================================================================
// Re-exports from events
// ============================================================================

pub use events::{EnvelopeVersion, RunEventEnvelope, RunEventPayload, RunEventTypeV0, RunEventV0};

// ============================================================================
// Helper functions
// ============================================================================

/// Creates a reference string for a node within a run.
pub fn run_node_ref(run_id: RunId, node_id: &NodeId) -> String {
    format!("{}:{}", run_id, node_id)
}
