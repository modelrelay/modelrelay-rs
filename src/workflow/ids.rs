//! Workflow identifier types.
//!
//! This module re-exports strongly-typed identifiers from the generated OpenAPI types:
//! - `RunId` - Unique identifier for a workflow run (UUID)
//! - `NodeId` - Identifier for a node within a workflow (validated pattern)
//! - `ModelId` - Identifier for an LLM model
//! - `RequestId` - Identifier for an LLM request within a run (UUID)
//! - `PlanHash` - SHA-256 hash of the compiled workflow plan (64 hex chars)
//! - `Sha256Hash` - Generic SHA-256 hash for payload verification (64 hex chars)
//!
//! These types are generated from the OpenAPI spec with built-in validation.
//! Use `.parse()` or `TryFrom` to create them from strings.
//!
//! # Example
//!
//! ```ignore
//! use modelrelay::workflow::{NodeId, RunId};
//!
//! // NodeId validates pattern: ^[a-z][a-z0-9_]*$
//! let node_id: NodeId = "my_node".parse().expect("valid node id");
//!
//! // RunId wraps a UUID
//! let run_id = RunId::try_from(uuid::Uuid::new_v4()).unwrap();
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

// Re-export generated types (single source of truth from OpenAPI spec)
pub use crate::generated::{ModelId, NodeId, PlanHash, RequestId, RunId, Sha256Hash};

// ============================================================================
// Artifact key (SDK-only, not in OpenAPI)
// ============================================================================

/// Artifact key type for node outputs and run outputs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct ArtifactKey(String);

impl ArtifactKey {
    pub const NODE_OUTPUT_V0: &'static str = "node_output.v0";
    pub const RUN_OUTPUTS_V0: &'static str = "run_outputs.v0";

    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().trim().to_string())
    }

    pub fn node_output_v0() -> Self {
        Self(Self::NODE_OUTPUT_V0.to_string())
    }

    pub fn run_outputs_v0() -> Self {
        Self(Self::RUN_OUTPUTS_V0.to_string())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for ArtifactKey {
    fn from(value: &str) -> Self {
        ArtifactKey::new(value)
    }
}

impl From<String> for ArtifactKey {
    fn from(value: String) -> Self {
        ArtifactKey::new(value)
    }
}

impl From<ArtifactKey> for String {
    fn from(value: ArtifactKey) -> Self {
        value.0
    }
}

impl fmt::Display for ArtifactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
