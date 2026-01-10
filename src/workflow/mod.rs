//! Workflow runtime types for the ModelRelay workflow engine.
//!
//! This module contains types related to workflow execution:
//!
//! - **[`ids`]** - Identifier types (RunId, NodeId, RequestId, etc.)
//! - **[`run`]** - Run status and result types (RunStatusV0, NodeResultV0, etc.)
//! - **[`events`]** - Event types for run streaming (RunEventV0, etc.)
//!
//! The workflow authoring types live in [`crate::workflow_intent`].

pub mod events;
pub mod ids;
pub mod run;
mod spec;

// ============================================================================
// Schema JSON
// ============================================================================

/// JSON Schema for run_event.v2 events.
pub const RUN_EVENT_V0_SCHEMA_JSON: &str = include_str!("../run_event.schema.json");

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

pub use spec::NodeTypeV1;

// ============================================================================
// Re-exports from run
// ============================================================================

#[allow(unused_imports)]
pub use run::{
    NodeErrorV0, NodeResultV0, NodeStatusV0, PayloadArtifactV0, PayloadInfoV0, RunCostLineItemV0,
    RunCostSummaryV0, RunStatusV0,
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
