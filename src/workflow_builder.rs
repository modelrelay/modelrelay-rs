use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errors::{Error, Result, ValidationError};
use crate::responses::ResponseBuilder;
use crate::types::InputItem;
use crate::workflow::{
    ConditionV1, EdgeV1, ExecutionV1, NodeId, NodeTypeV1, NodeV1, OutputRefV1, WorkflowKind,
    WorkflowSpecV1,
};

// =============================================================================
// Binding Target Validation
// =============================================================================

fn validate_binding_targets_v1(
    node_id: &NodeId,
    input: &[InputItem],
    bindings: &[LlmResponsesBindingV1],
) -> Result<()> {
    let pattern = Regex::new(r"^/input/(\d+)(?:/content/(\d+))?").unwrap();

    for (i, binding) in bindings.iter().enumerate() {
        let to = match &binding.to {
            Some(t) => t,
            None => continue,
        };

        if !to.starts_with("/input/") {
            continue;
        }

        let captures = match pattern.captures(to) {
            Some(c) => c,
            None => continue,
        };

        let msg_index: usize = captures
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);

        if msg_index >= input.len() {
            return Err(Error::Validation(ValidationError::new(format!(
                "node \"{}\" binding {}: targets {} but request only has {} messages (indices 0-{}); add placeholder messages or adjust binding target",
                node_id,
                i,
                to,
                input.len(),
                input.len().saturating_sub(1)
            ))));
        }

        if let Some(content_match) = captures.get(2) {
            let content_index: usize = content_match.as_str().parse().unwrap_or(0);
            let content_len = match &input[msg_index] {
                InputItem::Message { content, .. } => content.len(),
            };
            if content_index >= content_len {
                return Err(Error::Validation(ValidationError::new(format!(
                    "node \"{}\" binding {}: targets {} but message {} only has {} content blocks (indices 0-{})",
                    node_id,
                    i,
                    to,
                    msg_index,
                    content_len,
                    content_len.saturating_sub(1)
                ))));
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmResponsesBindingEncodingV1 {
    Json,
    JsonString,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionModeV1 {
    Server,
    Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionV1 {
    pub mode: ToolExecutionModeV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LlmResponsesToolLimitsV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_llm_calls: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls_per_step: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_ttl_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmResponsesBindingV1 {
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<LlmResponsesBindingEncodingV1>,
}

impl LlmResponsesBindingV1 {
    pub fn json(from: NodeId, pointer: Option<String>, to: impl Into<String>) -> Self {
        Self {
            from,
            pointer,
            to: Some(to.into()),
            to_placeholder: None,
            encoding: None,
        }
    }

    pub fn json_string(from: NodeId, pointer: Option<String>, to: impl Into<String>) -> Self {
        Self {
            from,
            pointer,
            to: Some(to.into()),
            to_placeholder: None,
            encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
        }
    }

    pub fn placeholder(
        from: NodeId,
        pointer: Option<String>,
        placeholder_name: impl Into<String>,
    ) -> Self {
        Self {
            from,
            pointer,
            to: None,
            to_placeholder: Some(placeholder_name.into()),
            encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LlmResponsesNodeOptionsV1 {
    pub stream: Option<bool>,
    pub bindings: Option<Vec<LlmResponsesBindingV1>>,
    pub tool_execution: Option<ToolExecutionV1>,
    pub tool_limits: Option<LlmResponsesToolLimitsV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformJsonValueV1 {
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

impl TransformJsonValueV1 {
    pub fn new(from: NodeId, pointer: Option<String>) -> Self {
        Self { from, pointer }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformJsonInputV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<BTreeMap<String, TransformJsonValueV1>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<Vec<TransformJsonValueV1>>,
}

impl TransformJsonInputV1 {
    pub fn object(fields: BTreeMap<String, TransformJsonValueV1>) -> Self {
        Self {
            object: Some(fields),
            merge: None,
        }
    }

    pub fn merge(items: Vec<TransformJsonValueV1>) -> Self {
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

    pub fn validate_map_fanout(&self) -> Result<()> {
        self.validate()?;
        if let Some(ref object) = self.object {
            for (key, value) in object {
                if key.trim().is_empty() {
                    continue;
                }
                if value.from.as_str() != "item" {
                    return Err(Error::Validation(ValidationError::new(format!(
                        "map.fanout transform.json object.{key}.from must be \"item\""
                    ))));
                }
            }
        }
        if let Some(ref merge) = self.merge {
            for (idx, value) in merge.iter().enumerate() {
                if value.from.as_str() != "item" {
                    return Err(Error::Validation(ValidationError::new(format!(
                        "map.fanout transform.json merge[{idx}].from must be \"item\""
                    ))));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MapFanoutItemsV1 {
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MapFanoutItemBindingV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<LlmResponsesBindingEncodingV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MapFanoutSubNodeV1 {
    pub id: NodeId,
    #[serde(rename = "type")]
    pub node_type: NodeTypeV1,
    pub input: Value,
}

impl MapFanoutSubNodeV1 {
    pub fn llm_responses(
        id: NodeId,
        request: ResponseBuilder,
        options: LlmResponsesNodeOptionsV1,
    ) -> Result<Self> {
        let req = request.payload.into_request();
        req.validate(true)?;

        let mut input = json!({ "request": req });
        if let Some(s) = options.stream {
            input["stream"] = Value::Bool(s);
        }
        if let Some(exec) = options.tool_execution {
            let raw = serde_json::to_value(exec).map_err(|err| {
                Error::Validation(ValidationError::new(format!(
                    "invalid llm.responses tool_execution: {err}"
                )))
            })?;
            input["tool_execution"] = raw;
        }
        if let Some(limits) = options.tool_limits {
            let raw = serde_json::to_value(limits).map_err(|err| {
                Error::Validation(ValidationError::new(format!(
                    "invalid llm.responses tool_limits: {err}"
                )))
            })?;
            input["tool_limits"] = raw;
        }
        if let Some(bs) = options.bindings {
            if !bs.is_empty() {
                let raw = serde_json::to_value(bs).map_err(|err| {
                    Error::Validation(ValidationError::new(format!(
                        "invalid llm.responses bindings: {err}"
                    )))
                })?;
                input["bindings"] = raw;
            }
        }

        Ok(Self {
            id,
            node_type: NodeTypeV1::LlmResponses,
            input,
        })
    }

    pub fn route_switch(
        id: NodeId,
        request: ResponseBuilder,
        options: LlmResponsesNodeOptionsV1,
    ) -> Result<Self> {
        let req = request.payload.into_request();
        req.validate(true)?;

        let mut input = json!({ "request": req });
        if let Some(s) = options.stream {
            input["stream"] = Value::Bool(s);
        }
        if let Some(exec) = options.tool_execution {
            let raw = serde_json::to_value(exec).map_err(|err| {
                Error::Validation(ValidationError::new(format!(
                    "invalid route.switch tool_execution: {err}"
                )))
            })?;
            input["tool_execution"] = raw;
        }
        if let Some(limits) = options.tool_limits {
            let raw = serde_json::to_value(limits).map_err(|err| {
                Error::Validation(ValidationError::new(format!(
                    "invalid route.switch tool_limits: {err}"
                )))
            })?;
            input["tool_limits"] = raw;
        }
        if let Some(bs) = options.bindings {
            if !bs.is_empty() {
                let raw = serde_json::to_value(bs).map_err(|err| {
                    Error::Validation(ValidationError::new(format!(
                        "invalid route.switch bindings: {err}"
                    )))
                })?;
                input["bindings"] = raw;
            }
        }

        Ok(Self {
            id,
            node_type: NodeTypeV1::RouteSwitch,
            input,
        })
    }

    pub fn transform_json(id: NodeId, input: TransformJsonInputV1) -> Result<Self> {
        input.validate()?;
        let raw = serde_json::to_value(&input).map_err(|err| {
            Error::Validation(ValidationError::new(format!(
                "invalid transform.json input: {err}"
            )))
        })?;

        Ok(Self {
            id,
            node_type: NodeTypeV1::TransformJson,
            input: raw,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MapFanoutInputV1 {
    pub items: MapFanoutItemsV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_bindings: Option<Vec<MapFanoutItemBindingV1>>,
    pub subnode: MapFanoutSubNodeV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<i64>,
}

impl MapFanoutInputV1 {
    pub fn validate(&self) -> Result<()> {
        match self.subnode.node_type {
            NodeTypeV1::LlmResponses | NodeTypeV1::RouteSwitch => {
                if let Some(bindings) = self.subnode.input.get("bindings") {
                    if bindings.as_array().map(|b| !b.is_empty()).unwrap_or(false) {
                        return Err(Error::Validation(ValidationError::new(
                            "map.fanout subnode bindings are not allowed",
                        )));
                    }
                }
            }
            NodeTypeV1::TransformJson => {
                if self
                    .item_bindings
                    .as_ref()
                    .map(|b| !b.is_empty())
                    .unwrap_or(false)
                {
                    return Err(Error::Validation(ValidationError::new(
                        "map.fanout transform.json cannot use item_bindings",
                    )));
                }
                let input: TransformJsonInputV1 =
                    serde_json::from_value(self.subnode.input.clone()).map_err(|err| {
                        Error::Validation(ValidationError::new(format!(
                            "map.fanout transform.json input must be valid JSON: {err}"
                        )))
                    })?;
                input.validate_map_fanout()?;
            }
            _ => {
                return Err(Error::Validation(ValidationError::new(
                    "unsupported map.fanout subnode type",
                )))
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JoinAnyInputV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ConditionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JoinCollectInputV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ConditionV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowBuilderV1 {
    name: Option<String>,
    execution: Option<ExecutionV1>,
    nodes: Vec<NodeV1>,
    edges: Vec<EdgeV1>,
    outputs: Vec<OutputRefV1>,
}

impl WorkflowBuilderV1 {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        let trimmed = name.into();
        let out = trimmed.trim().to_string();
        self.name = if out.is_empty() { None } else { Some(out) };
        self
    }

    #[must_use]
    pub fn execution(mut self, exec: ExecutionV1) -> Self {
        self.execution = Some(exec);
        self
    }

    pub fn llm_responses(
        self,
        id: NodeId,
        request: ResponseBuilder,
        stream: Option<bool>,
    ) -> Result<Self> {
        self.llm_responses_with_options(id, request, stream, None, None, None)
    }

    pub fn llm_responses_with_bindings(
        self,
        id: NodeId,
        request: ResponseBuilder,
        stream: Option<bool>,
        bindings: Option<Vec<LlmResponsesBindingV1>>,
    ) -> Result<Self> {
        self.llm_responses_with_options(id, request, stream, bindings, None, None)
    }

    pub fn llm_responses_with_options(
        mut self,
        id: NodeId,
        request: ResponseBuilder,
        stream: Option<bool>,
        bindings: Option<Vec<LlmResponsesBindingV1>>,
        tool_execution: Option<ToolExecutionV1>,
        tool_limits: Option<LlmResponsesToolLimitsV1>,
    ) -> Result<Self> {
        let req = request.payload.into_request();
        req.validate(true)?;

        if let Some(ref bs) = bindings {
            if !bs.is_empty() {
                validate_binding_targets_v1(&id, &req.input, bs)?;
            }
        }

        let mut input = json!({ "request": req });
        if let Some(s) = stream {
            input["stream"] = Value::Bool(s);
        }
        if let Some(exec) = tool_execution {
            input["tool_execution"] = serde_json::to_value(exec).map_err(|err| {
                Error::Validation(ValidationError::new(format!(
                    "invalid llm.responses tool_execution: {err}"
                )))
            })?;
        }
        if let Some(limits) = tool_limits {
            input["tool_limits"] = serde_json::to_value(limits).map_err(|err| {
                Error::Validation(ValidationError::new(format!(
                    "invalid llm.responses tool_limits: {err}"
                )))
            })?;
        }
        if let Some(bs) = bindings {
            if !bs.is_empty() {
                input["bindings"] = serde_json::to_value(bs).map_err(|err| {
                    Error::Validation(ValidationError::new(format!(
                        "invalid llm.responses bindings: {err}"
                    )))
                })?;
            }
        }

        self.nodes.push(NodeV1 {
            id,
            node_type: NodeTypeV1::LlmResponses,
            input: Some(input),
        });
        Ok(self)
    }

    pub fn route_switch(
        mut self,
        id: NodeId,
        request: ResponseBuilder,
        stream: Option<bool>,
        bindings: Option<Vec<LlmResponsesBindingV1>>,
    ) -> Result<Self> {
        let req = request.payload.into_request();
        req.validate(true)?;

        if let Some(ref bs) = bindings {
            if !bs.is_empty() {
                validate_binding_targets_v1(&id, &req.input, bs)?;
            }
        }

        let mut input = json!({ "request": req });
        if let Some(s) = stream {
            input["stream"] = Value::Bool(s);
        }
        if let Some(bs) = bindings {
            if !bs.is_empty() {
                input["bindings"] = serde_json::to_value(bs).map_err(|err| {
                    Error::Validation(ValidationError::new(format!(
                        "invalid route.switch bindings: {err}"
                    )))
                })?;
            }
        }

        self.nodes.push(NodeV1 {
            id,
            node_type: NodeTypeV1::RouteSwitch,
            input: Some(input),
        });
        Ok(self)
    }

    #[must_use]
    pub fn join_all(mut self, id: NodeId) -> Self {
        self.nodes.push(NodeV1 {
            id,
            node_type: NodeTypeV1::JoinAll,
            input: None,
        });
        self
    }

    pub fn join_any(mut self, id: NodeId, input: Option<JoinAnyInputV1>) -> Result<Self> {
        let raw = match input {
            Some(val) => Some(serde_json::to_value(val).map_err(|err| {
                Error::Validation(ValidationError::new(format!(
                    "invalid join.any input: {err}"
                )))
            })?),
            None => None,
        };
        self.nodes.push(NodeV1 {
            id,
            node_type: NodeTypeV1::JoinAny,
            input: raw,
        });
        Ok(self)
    }

    pub fn join_collect(mut self, id: NodeId, input: JoinCollectInputV1) -> Result<Self> {
        let raw = serde_json::to_value(input).map_err(|err| {
            Error::Validation(ValidationError::new(format!(
                "invalid join.collect input: {err}"
            )))
        })?;
        self.nodes.push(NodeV1 {
            id,
            node_type: NodeTypeV1::JoinCollect,
            input: Some(raw),
        });
        Ok(self)
    }

    pub fn transform_json(mut self, id: NodeId, input: TransformJsonInputV1) -> Result<Self> {
        input.validate()?;
        let raw = serde_json::to_value(input).map_err(|err| {
            Error::Validation(ValidationError::new(format!(
                "invalid transform.json input: {err}"
            )))
        })?;
        self.nodes.push(NodeV1 {
            id,
            node_type: NodeTypeV1::TransformJson,
            input: Some(raw),
        });
        Ok(self)
    }

    pub fn map_fanout(mut self, id: NodeId, input: MapFanoutInputV1) -> Result<Self> {
        input.validate()?;
        let raw = serde_json::to_value(input).map_err(|err| {
            Error::Validation(ValidationError::new(format!(
                "invalid map.fanout input: {err}"
            )))
        })?;
        self.nodes.push(NodeV1 {
            id,
            node_type: NodeTypeV1::MapFanout,
            input: Some(raw),
        });
        Ok(self)
    }

    #[must_use]
    pub fn edge(mut self, from: NodeId, to: NodeId) -> Self {
        self.edges.push(EdgeV1 {
            from,
            to,
            when: None,
        });
        self
    }

    #[must_use]
    pub fn edge_when(mut self, from: NodeId, to: NodeId, when: ConditionV1) -> Self {
        self.edges.push(EdgeV1 {
            from,
            to,
            when: Some(when),
        });
        self
    }

    #[must_use]
    pub fn output(
        mut self,
        name: impl Into<String>,
        from: NodeId,
        pointer: Option<String>,
    ) -> Self {
        self.outputs.push(OutputRefV1 {
            name: name.into(),
            from,
            pointer,
        });
        self
    }

    pub fn build(self) -> Result<WorkflowSpecV1> {
        let mut edges = self.edges;
        edges.sort_by(|a, b| {
            let af = a.from.as_str();
            let bf = b.from.as_str();
            if af != bf {
                return af.cmp(bf);
            }
            let at = a.to.as_str();
            let bt = b.to.as_str();
            if at != bt {
                return at.cmp(bt);
            }
            let aw = a
                .when
                .as_ref()
                .map(|w| serde_json::to_string(w).unwrap_or_default())
                .unwrap_or_default();
            let bw = b
                .when
                .as_ref()
                .map(|w| serde_json::to_string(w).unwrap_or_default())
                .unwrap_or_default();
            aw.cmp(&bw)
        });

        let mut outputs = self.outputs;
        outputs.sort_by(|a, b| {
            let an = a.name.as_str();
            let bn = b.name.as_str();
            if an != bn {
                return an.cmp(bn);
            }
            let af = a.from.as_str();
            let bf = b.from.as_str();
            if af != bf {
                return af.cmp(bf);
            }
            let ap = a.pointer.as_deref().unwrap_or("");
            let bp = b.pointer.as_deref().unwrap_or("");
            ap.cmp(bp)
        });

        Ok(WorkflowSpecV1 {
            kind: WorkflowKind::WorkflowV1,
            name: self.name,
            execution: self.execution,
            nodes: self.nodes,
            edges: if edges.is_empty() { None } else { Some(edges) },
            outputs,
        })
    }
}

pub fn workflow_v1() -> WorkflowBuilderV1 {
    WorkflowBuilderV1::new()
}
