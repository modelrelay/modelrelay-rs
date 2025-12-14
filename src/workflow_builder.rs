use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errors::{Error, Result, ValidationError};
use crate::responses::ResponseBuilder;
use crate::types::ResponseRequest;
use crate::workflow::{
    EdgeV0, ExecutionV0, NodeId, NodeTypeV0, NodeV0, OutputRefV0, WorkflowKind, WorkflowSpecV0,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmResponsesBindingEncodingV0 {
    Json,
    JsonString,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmResponsesBindingV0 {
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<LlmResponsesBindingEncodingV0>,
}

impl LlmResponsesBindingV0 {
    pub fn json(from: impl Into<NodeId>, pointer: Option<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            pointer,
            to: to.into(),
            encoding: None,
        }
    }

    pub fn json_string(
        from: impl Into<NodeId>,
        pointer: Option<String>,
        to: impl Into<String>,
    ) -> Self {
        Self {
            from: from.into(),
            pointer,
            to: to.into(),
            encoding: Some(LlmResponsesBindingEncodingV0::JsonString),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformJsonValueV0 {
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

impl TransformJsonValueV0 {
    pub fn new(from: impl Into<NodeId>, pointer: Option<String>) -> Self {
        Self {
            from: from.into(),
            pointer,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformJsonInputV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<BTreeMap<String, TransformJsonValueV0>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<Vec<TransformJsonValueV0>>,
}

impl TransformJsonInputV0 {
    pub fn object(fields: BTreeMap<String, TransformJsonValueV0>) -> Self {
        Self {
            object: Some(fields),
            merge: None,
        }
    }

    pub fn merge(items: Vec<TransformJsonValueV0>) -> Self {
        Self {
            object: None,
            merge: Some(items),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let has_object = self.object.as_ref().map(|m| !m.is_empty()).unwrap_or(false);
        let has_merge = self.merge.as_ref().map(|m| !m.is_empty()).unwrap_or(false);

        match (has_object, has_merge) {
            (true, false) => Ok(()),
            (false, true) => Ok(()),
            (false, false) => Err(Error::Validation(ValidationError::new(
                "transform.json input requires either object or merge",
            ))),
            (true, true) => Err(Error::Validation(ValidationError::new(
                "transform.json input must not set both object and merge",
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkflowBuildIssueCode {
    DuplicateNodeId,
    DuplicateEdge,
    EdgeFromUnknownNode,
    EdgeToUnknownNode,
    DuplicateOutputName,
    OutputFromUnknownNode,
    MissingNodes,
    MissingOutputs,
}

impl WorkflowBuildIssueCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowBuildIssueCode::DuplicateNodeId => "duplicate_node_id",
            WorkflowBuildIssueCode::DuplicateEdge => "duplicate_edge",
            WorkflowBuildIssueCode::EdgeFromUnknownNode => "edge_from_unknown_node",
            WorkflowBuildIssueCode::EdgeToUnknownNode => "edge_to_unknown_node",
            WorkflowBuildIssueCode::DuplicateOutputName => "duplicate_output_name",
            WorkflowBuildIssueCode::OutputFromUnknownNode => "output_from_unknown_node",
            WorkflowBuildIssueCode::MissingNodes => "missing_nodes",
            WorkflowBuildIssueCode::MissingOutputs => "missing_outputs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowBuildIssue {
    pub code: WorkflowBuildIssueCode,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowBuildError {
    pub issues: Vec<WorkflowBuildIssue>,
}

impl std::fmt::Display for WorkflowBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.issues.is_empty() {
            return write!(f, "invalid workflow.v0 spec");
        }
        if self.issues.len() == 1 {
            return write!(f, "{}", self.issues[0].message);
        }
        write!(
            f,
            "{} (and {} more)",
            self.issues[0].message,
            self.issues.len() - 1
        )
    }
}

impl std::error::Error for WorkflowBuildError {}

pub fn validate_workflow_spec_v0(spec: &WorkflowSpecV0) -> Vec<WorkflowBuildIssue> {
    let mut issues: Vec<WorkflowBuildIssue> = Vec::new();

    if spec.nodes.is_empty() {
        issues.push(WorkflowBuildIssue {
            code: WorkflowBuildIssueCode::MissingNodes,
            message: "at least one node is required".to_string(),
        });
    }
    if spec.outputs.is_empty() {
        issues.push(WorkflowBuildIssue {
            code: WorkflowBuildIssueCode::MissingOutputs,
            message: "at least one output is required".to_string(),
        });
    }

    let mut nodes_by_id: HashSet<String> = HashSet::new();
    let mut dupes: HashSet<String> = HashSet::new();
    for n in &spec.nodes {
        let id = n.id.as_str().trim();
        if id.is_empty() {
            continue;
        }
        if nodes_by_id.contains(id) {
            dupes.insert(id.to_string());
        } else {
            nodes_by_id.insert(id.to_string());
        }
    }
    for id in dupes {
        issues.push(WorkflowBuildIssue {
            code: WorkflowBuildIssueCode::DuplicateNodeId,
            message: format!("duplicate node id: {}", id),
        });
    }

    let mut edges: HashSet<(String, String)> = HashSet::new();
    if let Some(spec_edges) = &spec.edges {
        for e in spec_edges {
            let from = e.from.as_str().trim().to_string();
            let to = e.to.as_str().trim().to_string();
            if !from.is_empty() && !nodes_by_id.contains(&from) {
                issues.push(WorkflowBuildIssue {
                    code: WorkflowBuildIssueCode::EdgeFromUnknownNode,
                    message: format!("edge from unknown node: {}", from),
                });
            }
            if !to.is_empty() && !nodes_by_id.contains(&to) {
                issues.push(WorkflowBuildIssue {
                    code: WorkflowBuildIssueCode::EdgeToUnknownNode,
                    message: format!("edge to unknown node: {}", to),
                });
            }
            if !edges.insert((from.clone(), to.clone())) {
                issues.push(WorkflowBuildIssue {
                    code: WorkflowBuildIssueCode::DuplicateEdge,
                    message: format!("duplicate edge: {} -> {}", from, to),
                });
            }
        }
    }

    let mut output_names: HashSet<String> = HashSet::new();
    let mut output_dupes: HashSet<String> = HashSet::new();
    for o in &spec.outputs {
        let name = o.name.trim().to_string();
        if !name.is_empty() {
            if output_names.contains(&name) {
                output_dupes.insert(name.clone());
            } else {
                output_names.insert(name);
            }
        }
        let from = o.from.as_str().trim().to_string();
        if !from.is_empty() && !nodes_by_id.contains(&from) {
            issues.push(WorkflowBuildIssue {
                code: WorkflowBuildIssueCode::OutputFromUnknownNode,
                message: format!("output from unknown node: {}", from),
            });
        }
    }
    for name in output_dupes {
        issues.push(WorkflowBuildIssue {
            code: WorkflowBuildIssueCode::DuplicateOutputName,
            message: format!("duplicate output name: {}", name),
        });
    }

    issues
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowBuilderV0 {
    name: Option<String>,
    execution: Option<ExecutionV0>,
    nodes: Vec<NodeV0>,
    edges: Vec<EdgeV0>,
    outputs: Vec<OutputRefV0>,
}

impl WorkflowBuilderV0 {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        let trimmed = name.into();
        let trimmed = trimmed.trim().to_string();
        self.name = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };
        self
    }

    #[must_use]
    pub fn execution(mut self, exec: ExecutionV0) -> Self {
        self.execution = Some(exec);
        self
    }

    #[must_use]
    pub fn node(mut self, node: NodeV0) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn llm_responses(
        self,
        id: impl Into<NodeId>,
        request: ResponseBuilder,
        stream: Option<bool>,
    ) -> Result<Self> {
        self.llm_responses_with_bindings(id, request, stream, None)
    }

    pub fn llm_responses_with_bindings(
        self,
        id: impl Into<NodeId>,
        request: ResponseBuilder,
        stream: Option<bool>,
        bindings: Option<Vec<LlmResponsesBindingV0>>,
    ) -> Result<Self> {
        let id: NodeId = id.into();
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "node_id is required",
            )));
        }

        let req = ResponseRequest {
            provider: request.provider,
            model: request.model,
            max_output_tokens: request.max_output_tokens,
            temperature: request.temperature,
            output_format: request.output_format,
            input: request.input,
            stop: request.stop,
            tools: request.tools,
            tool_choice: request.tool_choice,
        };
        req.validate(true)?;

        let mut input = json!({ "request": req });
        if let Some(s) = stream {
            input["stream"] = Value::Bool(s);
        }
        if let Some(bs) = bindings {
            if !bs.is_empty() {
                let raw = serde_json::to_value(bs).map_err(|err| {
                    Error::Validation(ValidationError::new(format!(
                        "invalid llm.responses bindings: {err}"
                    )))
                })?;
                input["bindings"] = raw;
            }
        }

        Ok(self.node(NodeV0 {
            id,
            node_type: NodeTypeV0::LlmResponses,
            input: Some(input),
        }))
    }

    #[must_use]
    pub fn join_all(self, id: impl Into<NodeId>) -> Self {
        self.node(NodeV0 {
            id: id.into(),
            node_type: NodeTypeV0::JoinAll,
            input: None,
        })
    }

    pub fn transform_json(
        self,
        id: impl Into<NodeId>,
        input: TransformJsonInputV0,
    ) -> Result<Self> {
        let id: NodeId = id.into();
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "node_id is required",
            )));
        }
        input.validate()?;
        let raw = serde_json::to_value(&input).map_err(|err| {
            Error::Validation(ValidationError::new(format!(
                "invalid transform.json input: {err}"
            )))
        })?;

        Ok(self.node(NodeV0 {
            id,
            node_type: NodeTypeV0::TransformJson,
            input: Some(raw),
        }))
    }

    #[must_use]
    pub fn edge(mut self, from: impl Into<NodeId>, to: impl Into<NodeId>) -> Self {
        self.edges.push(EdgeV0 {
            from: from.into(),
            to: to.into(),
        });
        self
    }

    #[must_use]
    pub fn output(
        mut self,
        name: impl Into<String>,
        from: impl Into<NodeId>,
        pointer: Option<String>,
    ) -> Self {
        self.outputs.push(OutputRefV0 {
            name: name.into(),
            from: from.into(),
            pointer,
        });
        self
    }

    pub fn build_result(self) -> std::result::Result<WorkflowSpecV0, WorkflowBuildError> {
        let mut spec = WorkflowSpecV0 {
            kind: WorkflowKind::WorkflowV0,
            name: self.name,
            execution: self.execution,
            nodes: self.nodes,
            edges: if self.edges.is_empty() {
                None
            } else {
                Some(self.edges)
            },
            outputs: self.outputs,
        };

        if let Some(edges) = spec.edges.as_mut() {
            edges.sort_by(|a, b| {
                a.from
                    .as_str()
                    .cmp(b.from.as_str())
                    .then_with(|| a.to.as_str().cmp(b.to.as_str()))
            });
        }
        spec.outputs.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.from.as_str().cmp(b.from.as_str()))
                .then_with(|| {
                    a.pointer
                        .as_deref()
                        .unwrap_or("")
                        .cmp(b.pointer.as_deref().unwrap_or(""))
                })
        });

        let issues = validate_workflow_spec_v0(&spec);
        if !issues.is_empty() {
            return Err(WorkflowBuildError { issues });
        }
        Ok(spec)
    }

    pub fn build(self) -> Result<WorkflowSpecV0> {
        self.build_result().map_err(|err| {
            Error::Validation(ValidationError::new(err.to_string()).with_field("workflow.v0"))
        })
    }
}

pub fn workflow_v0() -> WorkflowBuilderV0 {
    WorkflowBuilderV0::new()
}
