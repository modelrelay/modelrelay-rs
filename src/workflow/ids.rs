//! Workflow identifier types.
//!
//! This module contains strongly-typed identifiers for workflow concepts:
//! - `RunId` - Unique identifier for a workflow run
//! - `NodeId` - Identifier for a node within a workflow
//! - `ModelId` - Identifier for an LLM model
//! - `RequestId` - Identifier for an LLM request within a run
//! - `PlanHash` - SHA-256 hash of the compiled workflow plan
//! - `Sha256Hash` - Generic SHA-256 hash for payload verification
//! - `ArtifactKey` - Key for node/run output artifacts

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::identifiers::{hex_hash_32, string_id_type, uuid_id_type};

// ============================================================================
// UUID-based identifiers
// ============================================================================

uuid_id_type!(RunId, "run_id");
uuid_id_type!(RequestId, "request_id");

// ============================================================================
// String-based identifiers
// ============================================================================

string_id_type!(NodeId);
string_id_type!(ModelId);

// ============================================================================
// Hash types
// ============================================================================

hex_hash_32!(PlanHash, "plan_hash");
hex_hash_32!(Sha256Hash, "sha256 hash");

// ============================================================================
// Artifact key
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

// Legacy constants for backwards compatibility
pub const ARTIFACT_KEY_NODE_OUTPUT_V0: &str = ArtifactKey::NODE_OUTPUT_V0;
pub const ARTIFACT_KEY_RUN_OUTPUTS_V0: &str = ArtifactKey::RUN_OUTPUTS_V0;
