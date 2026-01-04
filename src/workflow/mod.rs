//! Workflow types for the ModelRelay workflow engine.
//!
//! This module contains all types related to workflow definitions and execution:
//!
//! - **[`ids`]** - Identifier types (RunId, NodeId, RequestId, etc.)
//! - **[`spec`]** - Workflow specification types (SpecV1, NodeV1, etc.)
//! - **[`run`]** - Run status and result types (RunStatusV0, NodeResultV0, etc.)
//! - **[`events`]** - Event types for run streaming (RunEventV0, etc.)
//!
//! ## Clean type aliases
//!
//! This module exports clean type aliases without the `Workflow` prefix:
//! - `workflow::Kind` (alias for `WorkflowKind`)
//! - `workflow::SpecV1` (alias for `WorkflowSpecV1`)
//!
//! This matches the pattern used in the Go and TypeScript SDKs.
//!
//! ## Example
//!
//! ```ignore
//! use modelrelay::workflow::{Kind, SpecV1, RunId, NodeId, RunEventV0};
//!
//! // Create identifiers
//! let run_id = RunId::new();
//! let node_id: NodeId = "my_node".into();
//!
//! // Use clean type names
//! let spec = SpecV1 {
//!     kind: Kind::WorkflowV1,
//!     // ...
//! };
//!
//! // Parse a run event
//! let event: RunEventV0 = serde_json::from_str(json_str)?;
//! println!("Run {} seq {}", event.run_id(), event.seq());
//! ```

pub mod events;
pub mod helpers_v1;
pub mod ids;
pub mod run;
pub mod spec;

// ============================================================================
// Schema JSON
// ============================================================================

/// JSON Schema for workflow.v1 specifications.
pub const WORKFLOW_V1_SCHEMA_JSON: &str = include_str!("../workflow_v1.schema.json");

/// JSON Schema for run_event.v0 events.
pub const RUN_EVENT_V0_SCHEMA_JSON: &str = include_str!("../run_event_v0.schema.json");

// ============================================================================
// JSON Pointer Constants
// ============================================================================

/// JSON pointer to extract text content from an LLM response output.
/// Points to: output[0].content[0].text
pub const LLM_TEXT_OUTPUT: &str = "/output/0/content/0/text";

/// JSON pointer to inject text into the user message of an LLM request.
/// Points to: input[1].content[0].text (index 1 assumes a system message at index 0).
/// The pointer is relative to the request object (not the full node input).
pub const LLM_USER_MESSAGE_TEXT: &str = "/input/1/content/0/text";

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
    ConditionOpV1, ConditionSourceV1, ConditionV1, EdgeV1, ExecutionV1, NodeTypeV1, NodeV1,
    OutputRefV1, WorkflowKind, WorkflowSpecV1,
};

// Clean type aliases (no Workflow prefix) - matches Go and TypeScript SDK patterns
#[allow(unused_imports)]
pub use spec::{Kind, SpecV1};

// ============================================================================
// Re-exports from helpers_v1
// ============================================================================

pub use helpers_v1::{
    bind_to_placeholder, bind_to_placeholder_with_pointer, bind_to_pointer,
    bind_to_pointer_with_source, when_output_equals, when_output_exists, when_output_matches,
    when_status_equals, when_status_exists, when_status_matches, BindingBuilder,
};

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
