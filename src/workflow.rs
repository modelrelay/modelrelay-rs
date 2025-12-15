use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::errors::{Error, Result, ValidationError};

pub const WORKFLOW_V0_SCHEMA_JSON: &str = include_str!("workflow_v0.schema.json");
pub const RUN_EVENT_V0_SCHEMA_JSON: &str = include_str!("run_event_v0.schema.json");

// Re-export ProviderId from identifiers module (single source of truth).
pub use crate::identifiers::ProviderId;

/// Macro to generate string wrapper newtypes with consistent implementations.
/// Reduces boilerplate for NodeId, ModelId, etc.
macro_rules! string_id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into().trim().to_string())
            }

            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            pub fn is_empty(&self) -> bool {
                self.0.trim().is_empty()
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                $name::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                $name::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.as_str())
            }
        }
    };
}

/// Envelope version for run events. Schema specifies const "v0".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvelopeVersion {
    #[serde(rename = "v0")]
    #[default]
    V0,
}

impl EnvelopeVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            EnvelopeVersion::V0 => "v0",
        }
    }
}

impl fmt::Display for EnvelopeVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub Uuid);

impl RunId {
    /// Creates a new random RunId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates a RunId from an existing UUID.
    ///
    /// Useful for deterministic testing where you need predictable IDs.
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub fn parse(value: &str) -> Result<Self> {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "run_id is required",
            )));
        }
        let id = Uuid::parse_str(raw)
            .map_err(|err| Error::Validation(format!("invalid run_id: {err}").into()))?;
        Ok(Self(id))
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlanHash([u8; 32]);

impl PlanHash {
    pub fn parse(value: &str) -> Result<Self> {
        let raw = value.trim();
        if raw.len() != 64 {
            return Err(Error::Validation(ValidationError::new("invalid plan_hash")));
        }
        let bytes = hex::decode(raw)
            .map_err(|err| Error::Validation(format!("invalid plan_hash: {err}").into()))?;
        if bytes.len() != 32 {
            return Err(Error::Validation(ValidationError::new("invalid plan_hash")));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for PlanHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for PlanHash {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for PlanHash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(s.trim()).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("invalid plan_hash"));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }
}

// Generate string ID types using macro
string_id_type!(NodeId);
string_id_type!(ModelId);

/// Request ID for LLM calls, tool calls, and tool results.
/// Schema specifies format: uuid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub Uuid);

impl RequestId {
    /// Creates a new random RequestId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates a RequestId from an existing UUID.
    ///
    /// Useful for deterministic testing where you need predictable IDs.
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub fn parse(value: &str) -> Result<Self> {
        let raw = value.trim();
        if raw.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "request_id is required",
            )));
        }
        let id = Uuid::parse_str(raw)
            .map_err(|err| Error::Validation(format!("invalid request_id: {err}").into()))?;
        Ok(Self(id))
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// SHA-256 hash for payload integrity verification.
/// Schema specifies pattern: ^[0-9a-f]{64}$
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Hash([u8; 32]);

impl Sha256Hash {
    pub fn parse(value: &str) -> Result<Self> {
        let raw = value.trim();
        if raw.len() != 64 {
            return Err(Error::Validation(ValidationError::new(
                "invalid sha256 hash",
            )));
        }
        let bytes = hex::decode(raw)
            .map_err(|err| Error::Validation(format!("invalid sha256 hash: {err}").into()))?;
        if bytes.len() != 32 {
            return Err(Error::Validation(ValidationError::new(
                "invalid sha256 hash",
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl Serialize for Sha256Hash {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Sha256Hash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(s.trim()).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("invalid sha256 hash"));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeTypeV0 {
    #[serde(rename = "llm.responses")]
    LlmResponses,
    #[serde(rename = "join.all")]
    JoinAll,
    #[serde(rename = "transform.json")]
    TransformJson,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_timeout_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_timeout_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeV0 {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeTypeV0,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeV0 {
    pub from: NodeId,
    pub to: NodeId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputRefV0 {
    pub name: String,
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PayloadInfoV0 {
    pub bytes: i64,
    pub sha256: Sha256Hash,
    pub included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeErrorV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatusV0 {
    Running,
    Waiting,
    Succeeded,
    Failed,
    Canceled,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunCostSummaryV0 {
    pub total_usd_cents: i64,
    #[serde(default)]
    pub line_items: Vec<RunCostLineItemV0>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunCostLineItemV0 {
    pub provider_id: ProviderId,
    pub model: ModelId,
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub usd_cents: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RunEventTypeV0 {
    #[serde(rename = "run_compiled")]
    RunCompiled,
    #[serde(rename = "run_started")]
    RunStarted,
    #[serde(rename = "run_completed")]
    RunCompleted,
    #[serde(rename = "run_failed")]
    RunFailed,
    #[serde(rename = "run_canceled")]
    RunCanceled,
    #[serde(rename = "node_llm_call")]
    NodeLLMCall,
    #[serde(rename = "node_tool_call")]
    NodeToolCall,
    #[serde(rename = "node_tool_result")]
    NodeToolResult,
    #[serde(rename = "node_waiting")]
    NodeWaiting,
    #[serde(rename = "node_started")]
    NodeStarted,
    #[serde(rename = "node_succeeded")]
    NodeSucceeded,
    #[serde(rename = "node_failed")]
    NodeFailed,
    #[serde(rename = "node_output_delta")]
    NodeOutputDelta,
    #[serde(rename = "node_output")]
    NodeOutput,
}

/// Stream event kind from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamEventKind {
    MessageStart,
    MessageDelta,
    MessageStop,
    ToolUseStart,
    ToolUseDelta,
    ToolUseStop,
    /// Unknown event kind for forward compatibility.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeOutputDeltaV0 {
    pub kind: StreamEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenUsageV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeLLMCallV0 {
    pub step: i64,
    pub request_id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsageV0>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionToolCallV0 {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeToolCallV0 {
    pub step: i64,
    pub request_id: RequestId,
    pub tool_call: FunctionToolCallV0,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeToolResultV0 {
    pub step: i64,
    pub request_id: RequestId,
    pub tool_call_id: String,
    pub name: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingToolCallV0 {
    pub tool_call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeWaitingV0 {
    pub step: i64,
    pub request_id: RequestId,
    pub pending_tool_calls: Vec<PendingToolCallV0>,
    pub reason: String,
}

/// Common envelope fields for all run events.
///
/// These fields are present in every event and can be accessed directly
/// via `event.envelope.run_id` instead of pattern matching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunEventEnvelope {
    #[serde(default)]
    pub envelope_version: EnvelopeVersion,
    pub run_id: RunId,
    pub seq: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
}

/// Event-specific payload data.
///
/// Each variant contains only the fields unique to that event type.
/// Common fields (envelope_version, run_id, seq, ts) are in `RunEventEnvelope`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum RunEventPayload {
    #[serde(rename = "run_compiled")]
    RunCompiled { plan_hash: PlanHash },

    #[serde(rename = "run_started")]
    RunStarted { plan_hash: PlanHash },

    #[serde(rename = "run_completed")]
    RunCompleted {
        plan_hash: PlanHash,
        outputs_artifact_key: ArtifactKey,
        outputs_info: PayloadInfoV0,
    },

    #[serde(rename = "run_failed")]
    RunFailed {
        plan_hash: PlanHash,
        error: NodeErrorV0,
    },

    #[serde(rename = "run_canceled")]
    RunCanceled {
        plan_hash: PlanHash,
        error: NodeErrorV0,
    },

    #[serde(rename = "node_started")]
    NodeStarted { node_id: NodeId },

    #[serde(rename = "node_succeeded")]
    NodeSucceeded { node_id: NodeId },

    #[serde(rename = "node_failed")]
    NodeFailed { node_id: NodeId, error: NodeErrorV0 },

    #[serde(rename = "node_llm_call")]
    NodeLLMCall {
        node_id: NodeId,
        llm_call: NodeLLMCallV0,
    },

    #[serde(rename = "node_tool_call")]
    NodeToolCall {
        node_id: NodeId,
        tool_call: NodeToolCallV0,
    },

    #[serde(rename = "node_tool_result")]
    NodeToolResult {
        node_id: NodeId,
        tool_result: NodeToolResultV0,
    },

    #[serde(rename = "node_waiting")]
    NodeWaiting {
        node_id: NodeId,
        waiting: NodeWaitingV0,
    },

    #[serde(rename = "node_output_delta")]
    NodeOutputDelta {
        node_id: NodeId,
        delta: NodeOutputDeltaV0,
    },

    #[serde(rename = "node_output")]
    NodeOutput {
        node_id: NodeId,
        artifact_key: ArtifactKey,
        output_info: PayloadInfoV0,
    },
}

/// A run event with envelope metadata and payload.
///
/// The envelope contains common fields (envelope_version, run_id, seq, ts)
/// that are present in every event. The payload contains event-specific data.
///
/// # Example
/// ```ignore
/// // Direct field access (no pattern matching needed)
/// let run_id = event.envelope.run_id;
/// let seq = event.envelope.seq;
///
/// // Pattern match only for payload-specific data
/// match &event.payload {
///     RunEventPayload::RunCompleted { outputs_info, .. } => {
///         println!("Run completed with {} bytes", outputs_info.bytes);
///     }
///     _ => {}
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunEventV0 {
    /// Common envelope fields (envelope_version, run_id, seq, ts)
    #[serde(flatten)]
    pub envelope: RunEventEnvelope,
    /// Event-specific payload
    #[serde(flatten)]
    pub payload: RunEventPayload,
}

impl RunEventV0 {
    /// Returns the envelope version.
    pub fn envelope_version(&self) -> EnvelopeVersion {
        self.envelope.envelope_version
    }

    /// Returns the run ID.
    pub fn run_id(&self) -> &RunId {
        &self.envelope.run_id
    }

    /// Returns the sequence number.
    pub fn seq(&self) -> i64 {
        self.envelope.seq
    }

    /// Returns the timestamp.
    pub fn ts(&self) -> OffsetDateTime {
        self.envelope.ts
    }

    /// Validates the run event.
    /// Note: envelope_version is now an enum (EnvelopeVersion::V0), so invalid
    /// versions are caught at deserialization time rather than validation time.
    pub fn validate(&self) -> Result<()> {
        if self.envelope.seq < 1 {
            return Err(Error::Validation(ValidationError::new(
                "run event seq must be >= 1",
            )));
        }

        match &self.payload {
            RunEventPayload::NodeOutput { output_info, .. } => {
                if output_info.included {
                    return Err(Error::Validation(ValidationError::new(
                        "node_output output_info.included must be false",
                    )));
                }
            }
            RunEventPayload::RunCompleted { outputs_info, .. } => {
                if outputs_info.included {
                    return Err(Error::Validation(ValidationError::new(
                        "run_completed outputs_info.included must be false",
                    )));
                }
            }
            RunEventPayload::NodeOutputDelta { delta, .. } => {
                if delta.kind == StreamEventKind::Unknown {
                    return Err(Error::Validation(ValidationError::new(
                        "node_output_delta delta.kind is required",
                    )));
                }
            }
            RunEventPayload::NodeWaiting { waiting, .. } => {
                if waiting.step < 0 {
                    return Err(Error::Validation(ValidationError::new(
                        "node_waiting waiting.step must be >= 0",
                    )));
                }
                if waiting.request_id.0.is_nil() {
                    return Err(Error::Validation(ValidationError::new(
                        "node_waiting waiting.request_id is required",
                    )));
                }
                if waiting.pending_tool_calls.is_empty() {
                    return Err(Error::Validation(ValidationError::new(
                        "node_waiting waiting.pending_tool_calls is required",
                    )));
                }
                if waiting.reason.trim().is_empty() {
                    return Err(Error::Validation(ValidationError::new(
                        "node_waiting waiting.reason is required",
                    )));
                }
                for call in &waiting.pending_tool_calls {
                    if call.tool_call_id.trim().is_empty() || call.name.trim().is_empty() {
                        return Err(Error::Validation(ValidationError::new(
                            "node_waiting waiting.pending_tool_calls items must include tool_call_id and name",
                        )));
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
}

pub fn run_node_ref(run_id: RunId, node_id: &NodeId) -> String {
    format!("{}:{}", run_id, node_id)
}
