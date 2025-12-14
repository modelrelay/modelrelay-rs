use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::errors::{Error, Result, ValidationError};

pub const WORKFLOW_V0_SCHEMA_JSON: &str = include_str!("workflow_v0.schema.json");
pub const RUN_EVENT_V0_SCHEMA_JSON: &str = include_str!("run_event_v0.schema.json");

pub const ARTIFACT_KEY_NODE_OUTPUT_V0: &str = "node_output.v0";
pub const ARTIFACT_KEY_RUN_OUTPUTS_V0: &str = "run_outputs.v0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct NodeId(String);

impl NodeId {
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

impl From<&str> for NodeId {
    fn from(value: &str) -> Self {
        NodeId::new(value)
    }
}

impl From<String> for NodeId {
    fn from(value: String) -> Self {
        NodeId::new(value)
    }
}

impl From<NodeId> for String {
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
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
    pub sha256: String,
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
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatusV0 {
    Pending,
    Running,
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
    #[serde(rename = "node_started")]
    NodeStarted,
    #[serde(rename = "node_succeeded")]
    NodeSucceeded,
    #[serde(rename = "node_failed")]
    NodeFailed,
    #[serde(rename = "node_output")]
    NodeOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum RunEventV0 {
    #[serde(rename = "run_compiled")]
    RunCompiled {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        plan_hash: PlanHash,
    },

    #[serde(rename = "run_started")]
    RunStarted {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        plan_hash: PlanHash,
    },

    #[serde(rename = "run_completed")]
    RunCompleted {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        plan_hash: PlanHash,
        outputs_artifact_key: String,
        outputs_info: PayloadInfoV0,
    },

    #[serde(rename = "run_failed")]
    RunFailed {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        plan_hash: PlanHash,
        error: NodeErrorV0,
    },

    #[serde(rename = "run_canceled")]
    RunCanceled {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        plan_hash: PlanHash,
        error: NodeErrorV0,
    },

    #[serde(rename = "node_started")]
    NodeStarted {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        node_id: NodeId,
    },

    #[serde(rename = "node_succeeded")]
    NodeSucceeded {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        node_id: NodeId,
    },

    #[serde(rename = "node_failed")]
    NodeFailed {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        node_id: NodeId,
        error: NodeErrorV0,
    },

    #[serde(rename = "node_output")]
    NodeOutput {
        #[serde(default = "default_run_event_envelope_version")]
        envelope_version: String,
        run_id: RunId,
        seq: i64,
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        node_id: NodeId,
        artifact_key: String,
        output_info: PayloadInfoV0,
    },
}

impl RunEventV0 {
    pub fn envelope_version(&self) -> &str {
        match self {
            RunEventV0::RunCompiled {
                envelope_version, ..
            }
            | RunEventV0::RunStarted {
                envelope_version, ..
            }
            | RunEventV0::RunCompleted {
                envelope_version, ..
            }
            | RunEventV0::RunFailed {
                envelope_version, ..
            }
            | RunEventV0::RunCanceled {
                envelope_version, ..
            }
            | RunEventV0::NodeStarted {
                envelope_version, ..
            }
            | RunEventV0::NodeSucceeded {
                envelope_version, ..
            }
            | RunEventV0::NodeFailed {
                envelope_version, ..
            }
            | RunEventV0::NodeOutput {
                envelope_version, ..
            } => envelope_version,
        }
    }

    pub fn run_id(&self) -> &RunId {
        match self {
            RunEventV0::RunCompiled { run_id, .. }
            | RunEventV0::RunStarted { run_id, .. }
            | RunEventV0::RunCompleted { run_id, .. }
            | RunEventV0::RunFailed { run_id, .. }
            | RunEventV0::RunCanceled { run_id, .. }
            | RunEventV0::NodeStarted { run_id, .. }
            | RunEventV0::NodeSucceeded { run_id, .. }
            | RunEventV0::NodeFailed { run_id, .. }
            | RunEventV0::NodeOutput { run_id, .. } => run_id,
        }
    }

    pub fn seq(&self) -> i64 {
        match self {
            RunEventV0::RunCompiled { seq, .. }
            | RunEventV0::RunStarted { seq, .. }
            | RunEventV0::RunCompleted { seq, .. }
            | RunEventV0::RunFailed { seq, .. }
            | RunEventV0::RunCanceled { seq, .. }
            | RunEventV0::NodeStarted { seq, .. }
            | RunEventV0::NodeSucceeded { seq, .. }
            | RunEventV0::NodeFailed { seq, .. }
            | RunEventV0::NodeOutput { seq, .. } => *seq,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.envelope_version() != "v0" {
            return Err(Error::Validation(ValidationError::new(format!(
                "unsupported run event envelope_version: {}",
                self.envelope_version()
            ))));
        }
        if self.seq() < 1 {
            return Err(Error::Validation(ValidationError::new(
                "run event seq must be >= 1",
            )));
        }

        if let RunEventV0::NodeOutput { output_info, .. } = self {
            if output_info.included {
                return Err(Error::Validation(ValidationError::new(
                    "node_output output_info.included must be false",
                )));
            }
        }
        if let RunEventV0::RunCompleted { outputs_info, .. } = self {
            if outputs_info.included {
                return Err(Error::Validation(ValidationError::new(
                    "run_completed outputs_info.included must be false",
                )));
            }
        }

        Ok(())
    }
}

fn default_run_event_envelope_version() -> String {
    "v0".to_string()
}

pub fn run_node_ref(run_id: RunId, node_id: &NodeId) -> String {
    format!("{}:{}", run_id, node_id)
}
