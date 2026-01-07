//! Workflow run event types.
//!
//! This module contains types for workflow run events:
//! - `RunEventV0` - A run event with envelope and payload
//! - `RunEventEnvelope` - Common envelope fields for all events
//! - `RunEventPayload` - Event-specific payload data
//! - `RunEventTypeV0` - Type discriminator for events
//! - `StreamEventKind` - Kind of streaming event from LLM
//! - Various LLM call and tool call types

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{ModelId, NodeId, PlanHash, RequestId, RunId};
use super::run::{NodeErrorV0, PayloadArtifactV0};
use crate::errors::{Error, Result, ValidationError};
use crate::identifiers::ProviderId;

/// Envelope version for run events. Schema specifies const "v2".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvelopeVersion {
    #[serde(rename = "v2")]
    #[default]
    V2,
}

impl EnvelopeVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            EnvelopeVersion::V2 => "v2",
        }
    }
}

impl fmt::Display for EnvelopeVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Run event type discriminator.
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

/// Delta output from a streaming node.
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

/// Token usage for an LLM call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenUsageV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

/// LLM call event data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeLLMCallV0 {
    pub step: u64,
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

/// Tool call data (arguments optional).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallV0 {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Tool call data with required arguments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallWithArgumentsV0 {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Tool call event data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeToolCallV0 {
    pub step: u64,
    pub request_id: RequestId,
    pub tool_call: ToolCallWithArgumentsV0,
}

/// Tool result event data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeToolResultV0 {
    pub step: u64,
    pub request_id: RequestId,
    pub tool_call: ToolCallV0,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Pending tool call awaiting result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingToolCallV0 {
    pub tool_call: ToolCallWithArgumentsV0,
}

/// Node waiting event data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeWaitingV0 {
    pub step: u64,
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
    pub seq: u64,
    pub ts: DateTime<Utc>,
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
        outputs: PayloadArtifactV0,
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
        output: PayloadArtifactV0,
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
///     RunEventPayload::RunCompleted { outputs, .. } => {
///         println!("Run completed with {} bytes", outputs.info.bytes);
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
    pub fn seq(&self) -> u64 {
        self.envelope.seq
    }

    /// Returns the timestamp.
    pub fn ts(&self) -> DateTime<Utc> {
        self.envelope.ts
    }

    /// Validates the run event.
    /// Note: envelope_version is now an enum (EnvelopeVersion::V2), so invalid
    /// versions are caught at deserialization time rather than validation time.
    pub fn validate(&self) -> Result<()> {
        if self.envelope.seq < 1 {
            return Err(Error::Validation(ValidationError::new(
                "run event seq must be >= 1",
            )));
        }

        match &self.payload {
            RunEventPayload::NodeOutput { output, .. } => {
                if output.info.included {
                    return Err(Error::Validation(ValidationError::new(
                        "node_output output.info.included must be false",
                    )));
                }
            }
            RunEventPayload::RunCompleted { outputs, .. } => {
                if outputs.info.included {
                    return Err(Error::Validation(ValidationError::new(
                        "run_completed outputs.info.included must be false",
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
                // step is u64, so always >= 0
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
                    if call.tool_call.id.trim().is_empty() || call.tool_call.name.trim().is_empty()
                    {
                        return Err(Error::Validation(ValidationError::new(
                            "node_waiting waiting.pending_tool_calls items must include tool_call.id and tool_call.name",
                        )));
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
}
