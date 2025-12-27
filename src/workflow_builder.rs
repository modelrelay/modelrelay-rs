use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errors::{Error, Result, ValidationError};
use crate::responses::ResponseBuilder;
use crate::types::InputItem;
use crate::workflow::{
    EdgeV0, ExecutionV0, NodeId, NodeTypeV0, NodeV0, OutputRefV0, WorkflowKind, WorkflowSpecV0,
};

// =============================================================================
// Binding Target Validation
// =============================================================================

/// Validates that binding targets exist in the request input.
fn validate_binding_targets(
    node_id: &NodeId,
    input: &[InputItem],
    bindings: &[LlmResponsesBindingV0],
) -> Result<()> {
    // Compile regex once (could use lazy_static but this is fine for infrequent validation)
    let pattern = Regex::new(r"^/input/(\d+)(?:/content/(\d+))?").unwrap();

    for (i, binding) in bindings.iter().enumerate() {
        // Skip placeholder bindings (no `to` path to validate)
        let to = match &binding.to {
            Some(t) => t,
            None => continue,
        };

        // Only validate /input/... pointers
        if !to.starts_with("/input/") {
            continue;
        }

        let captures = match pattern.captures(to) {
            Some(c) => c,
            None => continue, // Doesn't match our pattern, skip validation
        };

        let msg_index: usize = captures
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);

        if msg_index >= input.len() {
            return Err(Error::Validation(ValidationError::new(format!(
                "node \"{}\" binding {}: targets {} but request only has {} messages (indices 0-{}); add placeholder messages or adjust binding target",
                node_id, i, to, input.len(), input.len().saturating_sub(1)
            ))));
        }

        // Optionally validate content block index
        if let Some(content_match) = captures.get(2) {
            let content_index: usize = content_match.as_str().parse().unwrap_or(0);
            let content_len = match &input[msg_index] {
                InputItem::Message { content, .. } => content.len(),
            };
            if content_index >= content_len {
                return Err(Error::Validation(ValidationError::new(format!(
                    "node \"{}\" binding {}: targets {} but message {} only has {} content blocks (indices 0-{})",
                    node_id, i, to, msg_index, content_len, content_len.saturating_sub(1)
                ))));
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmResponsesBindingEncodingV0 {
    Json,
    JsonString,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionModeV0 {
    Server,
    Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolExecutionV0 {
    pub mode: ToolExecutionModeV0,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LlmResponsesToolLimitsV0 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_llm_calls: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls_per_step: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_ttl_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmResponsesBindingV0 {
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<LlmResponsesBindingEncodingV0>,
}

impl LlmResponsesBindingV0 {
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
            encoding: Some(LlmResponsesBindingEncodingV0::JsonString),
        }
    }

    /// Creates a placeholder binding that injects a JSON-stringified value into a `{{name}}` marker.
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
            encoding: Some(LlmResponsesBindingEncodingV0::JsonString),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LlmResponsesNodeOptionsV0 {
    pub stream: Option<bool>,
    pub bindings: Option<Vec<LlmResponsesBindingV0>>,
    pub tool_execution: Option<ToolExecutionV0>,
    pub tool_limits: Option<LlmResponsesToolLimitsV0>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformJsonValueV0 {
    pub from: NodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

impl TransformJsonValueV0 {
    pub fn new(from: NodeId, pointer: Option<String>) -> Self {
        Self { from, pointer }
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
        id: NodeId,
        request: ResponseBuilder,
        stream: Option<bool>,
    ) -> Result<Self> {
        self.llm_responses_with_options(
            id,
            request,
            LlmResponsesNodeOptionsV0 {
                stream,
                ..Default::default()
            },
        )
    }

    pub fn llm_responses_with_bindings(
        self,
        id: NodeId,
        request: ResponseBuilder,
        stream: Option<bool>,
        bindings: Option<Vec<LlmResponsesBindingV0>>,
    ) -> Result<Self> {
        self.llm_responses_with_options(
            id,
            request,
            LlmResponsesNodeOptionsV0 {
                stream,
                bindings,
                ..Default::default()
            },
        )
    }

    pub fn llm_responses_with_options(
        self,
        id: NodeId,
        request: ResponseBuilder,
        options: LlmResponsesNodeOptionsV0,
    ) -> Result<Self> {
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "node_id is required",
            )));
        }

        // Validate binding targets before consuming the request
        if let Some(ref bindings) = options.bindings {
            if !bindings.is_empty() {
                validate_binding_targets(&id, &request.payload.input, bindings)?;
            }
        }

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

        Ok(self.node(NodeV0 {
            id,
            node_type: NodeTypeV0::LlmResponses,
            input: Some(input),
        }))
    }

    #[must_use]
    pub fn join_all(self, id: NodeId) -> Self {
        self.node(NodeV0 {
            id,
            node_type: NodeTypeV0::JoinAll,
            input: None,
        })
    }

    pub fn transform_json(self, id: NodeId, input: TransformJsonInputV0) -> Result<Self> {
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
    pub fn edge(mut self, from: NodeId, to: NodeId) -> Self {
        self.edges.push(EdgeV0 { from, to });
        self
    }

    #[must_use]
    pub fn output(
        mut self,
        name: impl Into<String>,
        from: NodeId,
        pointer: Option<String>,
    ) -> Self {
        self.outputs.push(OutputRefV0 {
            name: name.into(),
            from,
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

// =============================================================================
// Ergonomic Workflow Builder (auto-edge inference, fluent node configuration)
// =============================================================================

use std::collections::BTreeSet;

/// Semantic JSON pointer constants for LLM responses nodes.
/// These eliminate magic strings and make bindings self-documenting.
///
/// JSON pointer to extract text content from an LLM response output.
pub const LLM_TEXT_OUTPUT: &str = "/output/0/content/0/text";

/// JSON pointer to inject text into the user message of an LLM request.
/// The pointer is relative to the request object (not the full node input).
pub const LLM_USER_MESSAGE_TEXT: &str = "/input/1/content/0/text";

/// Build a JSON pointer to access a specific node's text output from a join.all node.
///
/// A join.all node produces an object keyed by upstream node IDs. This function
/// builds a pointer to extract the first text content from a specific upstream node.
///
/// # Example
///
/// ```ignore
/// // Access text from cost_analyst in a join output:
/// let pointer = join_output_text("cost_analyst");
/// // Produces: "/cost_analyst/output/0/content/0/text"
/// ```
pub fn join_output_text(node_id: &str) -> String {
    format!("/{}{}", node_id, LLM_TEXT_OUTPUT)
}

/// Pending LLM node configuration before it's finalized.
#[derive(Debug)]
struct PendingLlmNode {
    id: NodeId,
    request: Value,
    stream: Option<bool>,
    bindings: Vec<LlmResponsesBindingV0>,
    tool_execution: Option<ToolExecutionV0>,
    tool_limits: Option<LlmResponsesToolLimitsV0>,
}

/// Ergonomic workflow builder with auto-edge inference from bindings.
///
/// # Example
///
/// ```ignore
/// let spec = Workflow::new("tier_generation")
///     .add_llm_node("tier_generator", tier_req)?.stream(true)
///     .add_llm_node("business_summary", summary_req)?
///         .bind_from("tier_generator", Some("/output/0/content/0/text"))
///     .output("tiers", "tier_generator".into(), None)
///     .output("summary", "business_summary".into(), None)
///     .build()?;
/// ```
#[derive(Debug)]
pub struct Workflow {
    name: Option<String>,
    execution: Option<ExecutionV0>,
    nodes: Vec<NodeV0>,
    edges: BTreeSet<(NodeId, NodeId)>, // Deduplicated edges
    outputs: Vec<OutputRefV0>,
    pending_node: Option<PendingLlmNode>,
}

impl Workflow {
    /// Create a new workflow builder with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        let trimmed = name.into();
        let trimmed = trimmed.trim().to_string();
        Self {
            name: if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            },
            execution: None,
            nodes: Vec::new(),
            edges: BTreeSet::new(),
            outputs: Vec::new(),
            pending_node: None,
        }
    }

    /// Set the workflow execution configuration.
    #[must_use]
    pub fn execution(mut self, exec: ExecutionV0) -> Self {
        self.flush_pending_node();
        self.execution = Some(exec);
        self
    }

    /// Add an LLM responses node and return a node builder for configuration.
    pub fn add_llm_node(
        mut self,
        id: impl Into<NodeId>,
        request: ResponseBuilder,
    ) -> Result<LlmNodeBuilder> {
        self.flush_pending_node();

        let id = id.into();
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "node_id is required",
            )));
        }

        let req = request.payload.into_request();
        req.validate(true)?;
        let req_value = serde_json::to_value(&req).map_err(|e| {
            Error::Validation(ValidationError::new(format!("invalid request: {e}")))
        })?;

        self.pending_node = Some(PendingLlmNode {
            id,
            request: req_value,
            stream: None,
            bindings: Vec::new(),
            tool_execution: None,
            tool_limits: None,
        });

        Ok(LlmNodeBuilder { workflow: self })
    }

    /// Add a join.all node that waits for all incoming edges.
    #[must_use]
    pub fn add_join_all_node(mut self, id: impl Into<NodeId>) -> Self {
        self.flush_pending_node();
        self.nodes.push(NodeV0 {
            id: id.into(),
            node_type: NodeTypeV0::JoinAll,
            input: None,
        });
        self
    }

    /// Add a transform.json node and return a builder for configuration.
    pub fn add_transform_json_node(mut self, id: impl Into<NodeId>) -> TransformJsonNodeBuilder {
        self.flush_pending_node();
        TransformJsonNodeBuilder {
            workflow: self,
            id: id.into(),
            input: TransformJsonInputV0 {
                object: None,
                merge: None,
            },
        }
    }

    /// Add an output reference.
    #[must_use]
    pub fn output(
        mut self,
        name: impl Into<String>,
        from: NodeId,
        pointer: Option<String>,
    ) -> Self {
        self.flush_pending_node();
        self.outputs.push(OutputRefV0 {
            name: name.into(),
            from,
            pointer,
        });
        self
    }

    /// Add an output reference extracting text content from an LLM response.
    /// This is a convenience method that uses the LLM_TEXT_OUTPUT pointer.
    #[must_use]
    pub fn output_text(self, name: impl Into<String>, from: NodeId) -> Self {
        self.output(name, from, Some(LLM_TEXT_OUTPUT.to_string()))
    }

    /// Explicitly add an edge between nodes.
    /// Note: edges are automatically inferred from bindings, so this is rarely needed.
    #[must_use]
    pub fn edge(mut self, from: NodeId, to: NodeId) -> Self {
        self.flush_pending_node();
        self.edges.insert((from, to));
        self
    }

    /// Build the workflow specification.
    pub fn build(mut self) -> Result<WorkflowSpecV0> {
        self.flush_pending_node();

        // Convert edge set to sorted vec
        let edges: Vec<EdgeV0> = self
            .edges
            .into_iter()
            .map(|(from, to)| EdgeV0 { from, to })
            .collect();

        // Sort outputs
        let mut outputs = self.outputs;
        outputs.sort_by(|a, b| {
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

        Ok(WorkflowSpecV0 {
            kind: WorkflowKind::WorkflowV0,
            name: self.name,
            execution: self.execution,
            nodes: self.nodes,
            edges: if edges.is_empty() { None } else { Some(edges) },
            outputs,
        })
    }

    fn flush_pending_node(&mut self) {
        let pending = match self.pending_node.take() {
            Some(p) => p,
            None => return,
        };

        let mut input = json!({ "request": pending.request });
        if let Some(s) = pending.stream {
            input["stream"] = Value::Bool(s);
        }
        if let Some(exec) = pending.tool_execution {
            if let Ok(raw) = serde_json::to_value(exec) {
                input["tool_execution"] = raw;
            }
        }
        if let Some(limits) = pending.tool_limits {
            if let Ok(raw) = serde_json::to_value(limits) {
                input["tool_limits"] = raw;
            }
        }
        if !pending.bindings.is_empty() {
            if let Ok(raw) = serde_json::to_value(&pending.bindings) {
                input["bindings"] = raw;
            }
        }

        self.nodes.push(NodeV0 {
            id: pending.id.clone(),
            node_type: NodeTypeV0::LlmResponses,
            input: Some(input),
        });

        // Auto-infer edges from bindings
        for binding in &pending.bindings {
            self.edges
                .insert((binding.from.clone(), pending.id.clone()));
        }
    }
}

/// Builder for configuring an LLM responses node.
#[derive(Debug)]
pub struct LlmNodeBuilder {
    workflow: Workflow,
}

impl LlmNodeBuilder {
    /// Enable or disable streaming for this node.
    #[must_use]
    pub fn stream(mut self, enabled: bool) -> Self {
        if let Some(ref mut pending) = self.workflow.pending_node {
            pending.stream = Some(enabled);
        }
        self
    }

    /// Add a binding from another LLM node's text output to this node's user message.
    /// This is the most common binding pattern: LLM text → user message with json_string encoding.
    /// The edge from the source node is automatically inferred.
    #[must_use]
    pub fn bind_text_from(self, from: impl Into<NodeId>) -> Self {
        self.bind_from_to(
            from,
            Some(LLM_TEXT_OUTPUT),
            LLM_USER_MESSAGE_TEXT,
            Some(LlmResponsesBindingEncodingV0::JsonString),
        )
    }

    /// Add a binding from another node's output to this node's user message text.
    /// Use bind_text_from for the common case of binding LLM text output.
    /// The edge from the source node is automatically inferred.
    #[must_use]
    pub fn bind_from(self, from: impl Into<NodeId>, pointer: Option<&str>) -> Self {
        self.bind_from_to(
            from,
            pointer,
            LLM_USER_MESSAGE_TEXT,
            Some(LlmResponsesBindingEncodingV0::JsonString),
        )
    }

    /// Add a full binding with explicit source/destination pointers and encoding.
    /// The edge from the source node is automatically inferred.
    #[must_use]
    pub fn bind_from_to(
        mut self,
        from: impl Into<NodeId>,
        from_pointer: Option<&str>,
        to_pointer: &str,
        encoding: Option<LlmResponsesBindingEncodingV0>,
    ) -> Self {
        if let Some(ref mut pending) = self.workflow.pending_node {
            pending.bindings.push(LlmResponsesBindingV0 {
                from: from.into(),
                pointer: from_pointer.map(String::from),
                to: Some(to_pointer.to_string()),
                to_placeholder: None,
                encoding,
            });
        }
        self
    }

    /// Add a binding that replaces a {{placeholder}} in the prompt text.
    /// This is useful when the prompt contains placeholder markers like {{tier_data}}.
    /// The edge from the source node is automatically inferred.
    #[must_use]
    pub fn bind_to_placeholder(
        mut self,
        from: impl Into<NodeId>,
        from_pointer: Option<&str>,
        placeholder: impl Into<String>,
    ) -> Self {
        if let Some(ref mut pending) = self.workflow.pending_node {
            pending.bindings.push(LlmResponsesBindingV0 {
                from: from.into(),
                pointer: from_pointer.map(String::from),
                to: None,
                to_placeholder: Some(placeholder.into()),
                encoding: Some(LlmResponsesBindingEncodingV0::JsonString),
            });
        }
        self
    }

    /// Add a binding from an LLM node's text output to a placeholder.
    /// This is the most common placeholder binding: LLM text → {{placeholder}}.
    /// The edge from the source node is automatically inferred.
    #[must_use]
    pub fn bind_text_to_placeholder(
        self,
        from: impl Into<NodeId>,
        placeholder: impl Into<String>,
    ) -> Self {
        self.bind_to_placeholder(from, Some(LLM_TEXT_OUTPUT), placeholder)
    }

    /// Set the tool execution mode (server or client).
    #[must_use]
    pub fn tool_execution(mut self, mode: ToolExecutionModeV0) -> Self {
        if let Some(ref mut pending) = self.workflow.pending_node {
            pending.tool_execution = Some(ToolExecutionV0 { mode });
        }
        self
    }

    /// Set the tool execution limits.
    #[must_use]
    pub fn tool_limits(mut self, limits: LlmResponsesToolLimitsV0) -> Self {
        if let Some(ref mut pending) = self.workflow.pending_node {
            pending.tool_limits = Some(limits);
        }
        self
    }

    // Workflow methods for chaining back

    /// Add another LLM responses node.
    pub fn add_llm_node(
        self,
        id: impl Into<NodeId>,
        request: ResponseBuilder,
    ) -> Result<LlmNodeBuilder> {
        self.workflow.add_llm_node(id, request)
    }

    /// Add a join.all node.
    #[must_use]
    pub fn add_join_all_node(self, id: impl Into<NodeId>) -> Workflow {
        self.workflow.add_join_all_node(id)
    }

    /// Add a transform.json node.
    pub fn add_transform_json_node(self, id: impl Into<NodeId>) -> TransformJsonNodeBuilder {
        self.workflow.add_transform_json_node(id)
    }

    /// Add an explicit edge.
    #[must_use]
    pub fn edge(self, from: NodeId, to: NodeId) -> Workflow {
        self.workflow.edge(from, to)
    }

    /// Add an output reference.
    #[must_use]
    pub fn output(
        self,
        name: impl Into<String>,
        from: NodeId,
        pointer: Option<String>,
    ) -> Workflow {
        self.workflow.output(name, from, pointer)
    }

    /// Add an output reference extracting text content from an LLM response.
    #[must_use]
    pub fn output_text(self, name: impl Into<String>, from: NodeId) -> Workflow {
        self.workflow.output_text(name, from)
    }

    /// Set execution configuration.
    #[must_use]
    pub fn execution(self, exec: ExecutionV0) -> Workflow {
        self.workflow.execution(exec)
    }

    /// Build the workflow specification.
    pub fn build(self) -> Result<WorkflowSpecV0> {
        self.workflow.build()
    }
}

/// Builder for configuring a transform.json node.
#[derive(Debug)]
pub struct TransformJsonNodeBuilder {
    workflow: Workflow,
    id: NodeId,
    input: TransformJsonInputV0,
}

impl TransformJsonNodeBuilder {
    /// Set the object transformation with field mappings.
    #[must_use]
    pub fn object(mut self, fields: BTreeMap<String, TransformJsonValueV0>) -> Self {
        self.input.object = Some(fields);
        self
    }

    /// Set the merge transformation with source references.
    #[must_use]
    pub fn merge(mut self, items: Vec<TransformJsonValueV0>) -> Self {
        self.input.merge = Some(items);
        self
    }

    fn finalize(mut self) -> Workflow {
        let raw = serde_json::to_value(&self.input).ok();

        // Auto-infer edges from object field references
        if let Some(ref obj) = self.input.object {
            for ref_val in obj.values() {
                self.workflow
                    .edges
                    .insert((ref_val.from.clone(), self.id.clone()));
            }
        }

        // Auto-infer edges from merge references
        if let Some(ref merge) = self.input.merge {
            for ref_val in merge {
                self.workflow
                    .edges
                    .insert((ref_val.from.clone(), self.id.clone()));
            }
        }

        self.workflow.nodes.push(NodeV0 {
            id: self.id,
            node_type: NodeTypeV0::TransformJson,
            input: raw,
        });

        self.workflow
    }

    // Workflow methods for chaining back

    /// Add an LLM responses node.
    pub fn add_llm_node(
        self,
        id: impl Into<NodeId>,
        request: ResponseBuilder,
    ) -> Result<LlmNodeBuilder> {
        self.finalize().add_llm_node(id, request)
    }

    /// Add a join.all node.
    #[must_use]
    pub fn add_join_all_node(self, id: impl Into<NodeId>) -> Workflow {
        self.finalize().add_join_all_node(id)
    }

    /// Add an explicit edge.
    #[must_use]
    pub fn edge(self, from: NodeId, to: NodeId) -> Workflow {
        self.finalize().edge(from, to)
    }

    /// Add an output reference.
    #[must_use]
    pub fn output(
        self,
        name: impl Into<String>,
        from: NodeId,
        pointer: Option<String>,
    ) -> Workflow {
        self.finalize().output(name, from, pointer)
    }

    /// Set execution configuration.
    #[must_use]
    pub fn execution(self, exec: ExecutionV0) -> Workflow {
        self.finalize().execution(exec)
    }

    /// Build the workflow specification.
    pub fn build(self) -> Result<WorkflowSpecV0> {
        self.finalize().build()
    }
}

/// Create a new ergonomic workflow builder with the given name.
///
/// # Example
///
/// ```ignore
/// let spec = new_workflow("my_workflow")
///     .add_llm_node("generator", request)?.stream(true)
///     .add_llm_node("summarizer", summary_req)?
///         .bind_from("generator", Some("/output/0/content/0/text"))
///     .output("result", "summarizer".into(), None)
///     .build()?;
/// ```
pub fn new_workflow(name: impl Into<String>) -> Workflow {
    Workflow::new(name)
}
