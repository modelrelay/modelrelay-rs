use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errors::{Error, Result, ValidationError};
use crate::responses::ResponseBuilder;
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

        let req = request.payload.into_request();
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

    fn build_spec(self) -> WorkflowSpecV0 {
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
        spec
    }

    pub fn build(self) -> Result<WorkflowSpecV0> {
        Ok(self.build_spec())
    }
}

pub fn workflow_v0() -> WorkflowBuilderV0 {
    WorkflowBuilderV0::new()
}
