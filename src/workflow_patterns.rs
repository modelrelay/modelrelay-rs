//! High-level workflow pattern helpers.
//!
//! This module provides ergonomic builders for common workflow patterns:
//!
//! - [`Chain`] - Sequential LLM calls where each step's output feeds the next
//! - [`Parallel`] - Concurrent LLM calls with optional aggregation
//! - [`MapReduce`] - Parallel processing of items with a reducer
//!
//! These patterns abstract away the low-level DAG construction while
//! maintaining full control over the generated workflow specification.
//!
//! # Example
//!
//! ```ignore
//! use modelrelay::{Chain, Parallel, LLMStep, MapItem, MapReduce};
//!
//! // Chain: sequential processing
//! let spec = Chain::new("summarize-translate")
//!     .step(LLMStep::new("summarize", summarize_req))
//!     .step(LLMStep::new("translate", translate_req).with_stream())
//!     .output_last("result")
//!     .build()?;
//!
//! // Parallel: concurrent processing with aggregation
//! let spec = Parallel::new("multi-model")
//!     .step(LLMStep::new("gpt4", gpt4_req))
//!     .step(LLMStep::new("claude", claude_req))
//!     .aggregate("synthesize", synthesize_req)
//!     .output("result", "synthesize")
//!     .build()?;
//!
//! // MapReduce: parallel map with reduce
//! let spec = MapReduce::new("summarize-docs")
//!     .item(MapItem::new("doc1", doc1_req))
//!     .item(MapItem::new("doc2", doc2_req))
//!     .reduce("combine", combine_req)
//!     .output("result", "combine")
//!     .build()?;
//! ```

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::errors::{Error, Result, ValidationError};
use crate::responses::ResponseBuilder;
use crate::workflow::{
    EdgeV0, ExecutionV0, NodeId, NodeTypeV0, NodeV0, OutputRefV0, WorkflowKind, WorkflowSpecV0,
};
use crate::workflow_builder::{
    LlmResponsesBindingEncodingV0, LlmResponsesBindingV0, LLM_TEXT_OUTPUT, LLM_USER_MESSAGE_TEXT,
};

// =============================================================================
// LLMStep - Step configuration for Chain and Parallel patterns
// =============================================================================

/// Configuration for an LLM step in a workflow pattern.
///
/// Use [`LLMStep::new`] to create a step, then optionally call [`with_stream`](LLMStep::with_stream)
/// to enable streaming.
#[derive(Debug, Clone)]
pub struct LLMStep {
    id: NodeId,
    request: Value,
    stream: bool,
}

impl LLMStep {
    /// Create a new LLM step configuration.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this step within the workflow (must be a valid node ID)
    /// * `request` - The LLM request builder containing model, messages, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if the node ID is empty/invalid or the request is invalid.
    pub fn new(id: &str, request: ResponseBuilder) -> Result<Self> {
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "step ID cannot be empty",
            )));
        }

        let id: NodeId = id.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid step ID \"{}\": must start with lowercase letter and contain only lowercase letters, numbers, and underscores",
                id
            )))
        })?;

        let req = request.payload.into_request();
        req.validate(true)?;
        let request_value = serde_json::to_value(&req).map_err(|e| {
            Error::Validation(ValidationError::new(format!("invalid request: {e}")))
        })?;

        Ok(Self {
            id,
            request: request_value,
            stream: false,
        })
    }

    /// Enable streaming for this step.
    #[must_use]
    pub fn with_stream(mut self) -> Self {
        self.stream = true;
        self
    }

    /// Get the step's node ID.
    pub fn id(&self) -> &NodeId {
        &self.id
    }
}

// =============================================================================
// Chain - Sequential workflow pattern
// =============================================================================

/// Builder for sequential workflow patterns.
///
/// Creates a workflow where each step's text output is automatically bound
/// to the next step's user message input.
///
/// # Example
///
/// ```ignore
/// let spec = Chain::new("summarize-translate")
///     .step(LLMStep::new("summarize", summarize_req)?)
///     .step(LLMStep::new("translate", translate_req)?.with_stream())
///     .step(LLMStep::new("format", format_req)?)
///     .output_last("result")
///     .build()?;
/// ```
#[derive(Debug, Clone)]
pub struct Chain {
    name: String,
    steps: Vec<LLMStep>,
    execution: Option<ExecutionV0>,
    outputs: Vec<OutputRefV0>,
}

impl Chain {
    /// Create a new chain builder with the given workflow name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            execution: None,
            outputs: Vec::new(),
        }
    }

    /// Add a step to the chain.
    ///
    /// Steps are executed in order, with each step's text output bound
    /// to the next step's user message.
    #[must_use]
    pub fn step(mut self, step: LLMStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Set the workflow execution configuration.
    #[must_use]
    pub fn execution(mut self, exec: ExecutionV0) -> Self {
        self.execution = Some(exec);
        self
    }

    /// Add an output reference from a specific step.
    ///
    /// # Errors
    ///
    /// Returns an error if the `from` ID is not a valid node ID.
    pub fn output(mut self, name: impl Into<String>, from: &str) -> Result<Self> {
        let from_id: NodeId = from.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid output from ID \"{}\": must be a valid node ID",
                from
            )))
        })?;
        self.outputs.push(OutputRefV0 {
            name: name.into(),
            from: from_id,
            pointer: Some(LLM_TEXT_OUTPUT.to_string()),
        });
        Ok(self)
    }

    /// Add an output reference from the last step.
    ///
    /// This is a convenience method equivalent to calling `output(name, last_step_id)`.
    #[must_use]
    pub fn output_last(mut self, name: impl Into<String>) -> Self {
        if let Some(last) = self.steps.last() {
            self.outputs.push(OutputRefV0 {
                name: name.into(),
                from: last.id.clone(),
                pointer: Some(LLM_TEXT_OUTPUT.to_string()),
            });
        }
        self
    }

    /// Build the workflow specification.
    ///
    /// # Errors
    ///
    /// Returns an error if the chain has no steps.
    pub fn build(self) -> Result<WorkflowSpecV0> {
        if self.steps.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "chain requires at least one step",
            )));
        }

        let mut nodes = Vec::with_capacity(self.steps.len());
        let mut edges = Vec::new();

        for (i, step) in self.steps.iter().enumerate() {
            let mut input = json!({ "request": step.request });

            if step.stream {
                input["stream"] = Value::Bool(true);
            }

            // Bind from previous step (except for the first step)
            if i > 0 {
                let prev_id = &self.steps[i - 1].id;
                let binding = LlmResponsesBindingV0 {
                    from: prev_id.clone(),
                    pointer: Some(LLM_TEXT_OUTPUT.to_string()),
                    to: LLM_USER_MESSAGE_TEXT.to_string(),
                    encoding: Some(LlmResponsesBindingEncodingV0::JsonString),
                };
                let bindings = serde_json::to_value(vec![binding]).unwrap_or_default();
                input["bindings"] = bindings;

                edges.push(EdgeV0 {
                    from: prev_id.clone(),
                    to: step.id.clone(),
                });
            }

            nodes.push(NodeV0 {
                id: step.id.clone(),
                node_type: NodeTypeV0::LlmResponses,
                input: Some(input),
            });
        }

        // Sort outputs for deterministic output
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
            name: Some(self.name),
            execution: self.execution,
            nodes,
            edges: if edges.is_empty() { None } else { Some(edges) },
            outputs,
        })
    }
}

// =============================================================================
// Parallel - Parallel workflow pattern with optional aggregation
// =============================================================================

/// Configuration for the aggregator step in a parallel pattern.
#[derive(Debug, Clone)]
struct AggregateConfig {
    id: NodeId,
    request: Value,
    stream: bool,
}

/// Builder for parallel workflow patterns.
///
/// Creates a workflow where multiple LLM steps execute concurrently.
/// Optionally, an aggregator step can combine all outputs.
///
/// # Example
///
/// ```ignore
/// // Without aggregation - outputs from individual steps
/// let spec = Parallel::new("multi-model")
///     .step(LLMStep::new("gpt4", gpt4_req)?)
///     .step(LLMStep::new("claude", claude_req)?)
///     .output("gpt4_result", "gpt4")
///     .output("claude_result", "claude")
///     .build()?;
///
/// // With aggregation - combined output
/// let spec = Parallel::new("multi-model")
///     .step(LLMStep::new("gpt4", gpt4_req)?)
///     .step(LLMStep::new("claude", claude_req)?)
///     .aggregate("synthesize", synthesize_req)
///     .output("result", "synthesize")
///     .build()?;
/// ```
#[derive(Debug, Clone)]
pub struct Parallel {
    name: String,
    steps: Vec<LLMStep>,
    execution: Option<ExecutionV0>,
    aggregate: Option<AggregateConfig>,
    outputs: Vec<OutputRefV0>,
}

impl Parallel {
    /// Create a new parallel builder with the given workflow name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            execution: None,
            aggregate: None,
            outputs: Vec::new(),
        }
    }

    /// Add a step to run in parallel.
    #[must_use]
    pub fn step(mut self, step: LLMStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Set the workflow execution configuration.
    #[must_use]
    pub fn execution(mut self, exec: ExecutionV0) -> Self {
        self.execution = Some(exec);
        self
    }

    /// Add an aggregator that waits for all parallel steps and combines their outputs.
    ///
    /// This creates a join.all node followed by an LLM node that receives
    /// all outputs as a JSON object.
    ///
    /// The join node ID is automatically generated as `<id>_join`.
    pub fn aggregate(mut self, id: &str, request: ResponseBuilder) -> Result<Self> {
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "aggregate ID cannot be empty",
            )));
        }

        let id: NodeId = id.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid aggregate ID \"{}\": must be a valid node ID",
                id
            )))
        })?;

        let req = request.payload.into_request();
        req.validate(true)?;
        let request_value = serde_json::to_value(&req).map_err(|e| {
            Error::Validation(ValidationError::new(format!(
                "invalid aggregate request: {e}"
            )))
        })?;

        self.aggregate = Some(AggregateConfig {
            id,
            request: request_value,
            stream: false,
        });
        Ok(self)
    }

    /// Add an aggregator with streaming enabled.
    ///
    /// Like [`aggregate`](Self::aggregate), but enables streaming on the aggregator node.
    pub fn aggregate_with_stream(mut self, id: &str, request: ResponseBuilder) -> Result<Self> {
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "aggregate ID cannot be empty",
            )));
        }

        let id: NodeId = id.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid aggregate ID \"{}\": must be a valid node ID",
                id
            )))
        })?;

        let req = request.payload.into_request();
        req.validate(true)?;
        let request_value = serde_json::to_value(&req).map_err(|e| {
            Error::Validation(ValidationError::new(format!(
                "invalid aggregate request: {e}"
            )))
        })?;

        self.aggregate = Some(AggregateConfig {
            id,
            request: request_value,
            stream: true,
        });
        Ok(self)
    }

    /// Add an output reference from a specific step.
    ///
    /// # Errors
    ///
    /// Returns an error if the `from` ID is not a valid node ID.
    pub fn output(mut self, name: impl Into<String>, from: &str) -> Result<Self> {
        let from_id: NodeId = from.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid output from ID \"{}\": must be a valid node ID",
                from
            )))
        })?;
        self.outputs.push(OutputRefV0 {
            name: name.into(),
            from: from_id,
            pointer: Some(LLM_TEXT_OUTPUT.to_string()),
        });
        Ok(self)
    }

    /// Build the workflow specification.
    ///
    /// # Errors
    ///
    /// Returns an error if the parallel pattern has no steps.
    pub fn build(self) -> Result<WorkflowSpecV0> {
        if self.steps.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "parallel requires at least one step",
            )));
        }

        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Add all parallel nodes
        for step in &self.steps {
            let mut input = json!({ "request": step.request });
            if step.stream {
                input["stream"] = Value::Bool(true);
            }

            nodes.push(NodeV0 {
                id: step.id.clone(),
                node_type: NodeTypeV0::LlmResponses,
                input: Some(input),
            });
        }

        // Add join and aggregator if configured
        if let Some(agg) = &self.aggregate {
            let join_id: NodeId = format!("{}_join", agg.id.as_str()).parse().map_err(|_| {
                Error::Validation(ValidationError::new("failed to create join node ID"))
            })?;

            // Add join.all node
            nodes.push(NodeV0 {
                id: join_id.clone(),
                node_type: NodeTypeV0::JoinAll,
                input: None,
            });

            // Add edges from all parallel nodes to join
            for step in &self.steps {
                edges.push(EdgeV0 {
                    from: step.id.clone(),
                    to: join_id.clone(),
                });
            }

            // Add aggregator node with binding from join
            let mut agg_input = json!({ "request": agg.request });
            if agg.stream {
                agg_input["stream"] = Value::Bool(true);
            }

            let binding = LlmResponsesBindingV0 {
                from: join_id.clone(),
                pointer: None, // Empty pointer = full join output
                to: LLM_USER_MESSAGE_TEXT.to_string(),
                encoding: Some(LlmResponsesBindingEncodingV0::JsonString),
            };
            let bindings = serde_json::to_value(vec![binding]).unwrap_or_default();
            agg_input["bindings"] = bindings;

            nodes.push(NodeV0 {
                id: agg.id.clone(),
                node_type: NodeTypeV0::LlmResponses,
                input: Some(agg_input),
            });

            // Edge from join to aggregator
            edges.push(EdgeV0 {
                from: join_id,
                to: agg.id.clone(),
            });
        }

        // Sort edges for deterministic output
        edges.sort_by(|a, b| {
            a.from
                .as_str()
                .cmp(b.from.as_str())
                .then_with(|| a.to.as_str().cmp(b.to.as_str()))
        });

        // Sort outputs for deterministic output
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
            name: Some(self.name),
            execution: self.execution,
            nodes,
            edges: if edges.is_empty() { None } else { Some(edges) },
            outputs,
        })
    }
}

// =============================================================================
// MapItem - Item configuration for MapReduce pattern
// =============================================================================

/// Configuration for an item to be processed in a MapReduce pattern.
///
/// Each item becomes a separate mapper node that runs in parallel.
/// The item ID becomes part of the mapper node ID: `map_<id>`.
#[derive(Debug, Clone)]
pub struct MapItem {
    id: String,
    request: Value,
    stream: bool,
}

impl MapItem {
    /// Create a new map item configuration.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this item. Must be unique within the MapReduce workflow.
    /// * `request` - The LLM request builder for processing this item.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is empty or the request is invalid.
    pub fn new(id: impl Into<String>, request: ResponseBuilder) -> Result<Self> {
        let id = id.into();
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "item ID cannot be empty",
            )));
        }

        let req = request.payload.into_request();
        req.validate(true)?;
        let request_value = serde_json::to_value(&req).map_err(|e| {
            Error::Validation(ValidationError::new(format!("invalid item request: {e}")))
        })?;

        Ok(Self {
            id,
            request: request_value,
            stream: false,
        })
    }

    /// Enable streaming for this item's mapper node.
    #[must_use]
    pub fn with_stream(mut self) -> Self {
        self.stream = true;
        self
    }

    /// Get the item's ID.
    pub fn id(&self) -> &str {
        &self.id
    }
}

// =============================================================================
// MapReduce - Map-reduce workflow pattern
// =============================================================================

/// Configuration for the reducer step in a MapReduce pattern.
#[derive(Debug, Clone)]
struct ReducerConfig {
    id: NodeId,
    request: Value,
    stream: bool,
}

/// Builder for map-reduce workflow patterns.
///
/// Creates a workflow where items are processed in parallel by mapper nodes,
/// then combined by a reducer node.
///
/// The pattern creates:
/// - N mapper nodes (one per item), running in parallel
/// - A join.all node to collect all mapper outputs
/// - A reducer LLM node that receives the combined outputs
///
/// Note: Items must be known at workflow build time.
///
/// # Example
///
/// ```ignore
/// let spec = MapReduce::new("summarize-docs")
///     .item(MapItem::new("doc1", doc1_req)?)
///     .item(MapItem::new("doc2", doc2_req)?)
///     .item(MapItem::new("doc3", doc3_req)?.with_stream())
///     .reduce("combine", combine_req)
///     .output("result", "combine")
///     .build()?;
/// ```
#[derive(Debug, Clone)]
pub struct MapReduce {
    name: String,
    items: Vec<MapItem>,
    execution: Option<ExecutionV0>,
    reducer: Option<ReducerConfig>,
    outputs: Vec<OutputRefV0>,
}

impl MapReduce {
    /// Create a new map-reduce builder with the given workflow name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            items: Vec::new(),
            execution: None,
            reducer: None,
            outputs: Vec::new(),
        }
    }

    /// Add an item to be processed by a mapper.
    #[must_use]
    pub fn item(mut self, item: MapItem) -> Self {
        self.items.push(item);
        self
    }

    /// Add an item by ID and request (convenience method).
    ///
    /// # Example
    /// ```ignore
    /// let spec = MapReduce::new("summarize-docs")
    ///     .add_item("doc1", doc1_req)?
    ///     .add_item("doc2", doc2_req)?
    ///     .reduce("combine", combine_req)?
    ///     .build()?;
    /// ```
    pub fn add_item(self, id: &str, request: ResponseBuilder) -> Result<Self> {
        Ok(self.item(MapItem::new(id, request)?))
    }

    /// Add an item with streaming enabled (convenience method).
    pub fn add_item_with_stream(self, id: &str, request: ResponseBuilder) -> Result<Self> {
        Ok(self.item(MapItem::new(id, request)?.with_stream()))
    }

    /// Set the workflow execution configuration.
    #[must_use]
    pub fn execution(mut self, exec: ExecutionV0) -> Self {
        self.execution = Some(exec);
        self
    }

    /// Add a reducer that receives all mapper outputs.
    ///
    /// The reducer receives a JSON object mapping each mapper ID to its text output.
    /// The join node ID is automatically generated as `<id>_join`.
    pub fn reduce(mut self, id: &str, request: ResponseBuilder) -> Result<Self> {
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "reducer ID cannot be empty",
            )));
        }

        let id: NodeId = id.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid reducer ID \"{}\": must be a valid node ID",
                id
            )))
        })?;

        let req = request.payload.into_request();
        req.validate(true)?;
        let request_value = serde_json::to_value(&req).map_err(|e| {
            Error::Validation(ValidationError::new(format!(
                "invalid reducer request: {e}"
            )))
        })?;

        self.reducer = Some(ReducerConfig {
            id,
            request: request_value,
            stream: false,
        });
        Ok(self)
    }

    /// Add a reducer with streaming enabled.
    ///
    /// Like [`reduce`](Self::reduce), but enables streaming on the reducer node.
    pub fn reduce_with_stream(mut self, id: &str, request: ResponseBuilder) -> Result<Self> {
        if id.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "reducer ID cannot be empty",
            )));
        }

        let id: NodeId = id.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid reducer ID \"{}\": must be a valid node ID",
                id
            )))
        })?;

        let req = request.payload.into_request();
        req.validate(true)?;
        let request_value = serde_json::to_value(&req).map_err(|e| {
            Error::Validation(ValidationError::new(format!(
                "invalid reducer request: {e}"
            )))
        })?;

        self.reducer = Some(ReducerConfig {
            id,
            request: request_value,
            stream: true,
        });
        Ok(self)
    }

    /// Add an output reference from a specific node.
    ///
    /// Typically used to output from the reducer node.
    ///
    /// # Errors
    ///
    /// Returns an error if the `from` ID is not a valid node ID.
    pub fn output(mut self, name: impl Into<String>, from: &str) -> Result<Self> {
        let from_id: NodeId = from.parse().map_err(|_| {
            Error::Validation(ValidationError::new(format!(
                "invalid output from ID \"{}\": must be a valid node ID",
                from
            )))
        })?;
        self.outputs.push(OutputRefV0 {
            name: name.into(),
            from: from_id,
            pointer: Some(LLM_TEXT_OUTPUT.to_string()),
        });
        Ok(self)
    }

    /// Build the workflow specification.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The pattern has no items
    /// - No reducer is configured
    /// - There are duplicate item IDs
    pub fn build(self) -> Result<WorkflowSpecV0> {
        if self.items.is_empty() {
            return Err(Error::Validation(ValidationError::new(
                "map-reduce requires at least one item",
            )));
        }

        let reducer = self.reducer.ok_or_else(|| {
            Error::Validation(ValidationError::new(
                "map-reduce requires a reducer (call reduce)",
            ))
        })?;

        // Check for duplicate item IDs
        let mut seen_ids = HashSet::new();
        for item in &self.items {
            if !seen_ids.insert(&item.id) {
                return Err(Error::Validation(ValidationError::new(format!(
                    "duplicate item ID: \"{}\"",
                    item.id
                ))));
            }
        }

        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        let join_id: NodeId = format!("{}_join", reducer.id.as_str())
            .parse()
            .map_err(|_| {
                Error::Validation(ValidationError::new("failed to create join node ID"))
            })?;

        // Add mapper nodes
        for item in &self.items {
            let mapper_id: NodeId = format!("map_{}", item.id).parse().map_err(|_| {
                Error::Validation(ValidationError::new(format!(
                    "failed to create mapper node ID for item \"{}\"",
                    item.id
                )))
            })?;

            let mut input = json!({ "request": item.request });
            if item.stream {
                input["stream"] = Value::Bool(true);
            }

            nodes.push(NodeV0 {
                id: mapper_id.clone(),
                node_type: NodeTypeV0::LlmResponses,
                input: Some(input),
            });

            // Edge from mapper to join
            edges.push(EdgeV0 {
                from: mapper_id,
                to: join_id.clone(),
            });
        }

        // Add join.all node
        nodes.push(NodeV0 {
            id: join_id.clone(),
            node_type: NodeTypeV0::JoinAll,
            input: None,
        });

        // Add reducer node with binding from join
        let mut reducer_input = json!({ "request": reducer.request });
        if reducer.stream {
            reducer_input["stream"] = Value::Bool(true);
        }

        let binding = LlmResponsesBindingV0 {
            from: join_id.clone(),
            pointer: None, // Empty pointer = full join output
            to: LLM_USER_MESSAGE_TEXT.to_string(),
            encoding: Some(LlmResponsesBindingEncodingV0::JsonString),
        };
        let bindings = serde_json::to_value(vec![binding]).unwrap_or_default();
        reducer_input["bindings"] = bindings;

        nodes.push(NodeV0 {
            id: reducer.id.clone(),
            node_type: NodeTypeV0::LlmResponses,
            input: Some(reducer_input),
        });

        // Edge from join to reducer
        edges.push(EdgeV0 {
            from: join_id,
            to: reducer.id.clone(),
        });

        // Sort edges for deterministic output
        edges.sort_by(|a, b| {
            a.from
                .as_str()
                .cmp(b.from.as_str())
                .then_with(|| a.to.as_str().cmp(b.to.as_str()))
        });

        // Sort outputs for deterministic output
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
            name: Some(self.name),
            execution: self.execution,
            nodes,
            edges: if edges.is_empty() { None } else { Some(edges) },
            outputs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResponseBuilder;

    fn make_request(text: &str) -> ResponseBuilder {
        ResponseBuilder::new().model("echo-1").user(text)
    }

    mod llm_step {
        use super::*;

        #[test]
        fn creates_step_configuration() {
            let step = LLMStep::new("step1", make_request("hello")).unwrap();
            assert_eq!(step.id.as_str(), "step1");
            assert!(!step.stream);
        }

        #[test]
        fn with_stream_enables_streaming() {
            let step = LLMStep::new("step1", make_request("hello"))
                .unwrap()
                .with_stream();
            assert!(step.stream);
        }

        #[test]
        fn empty_id_returns_error() {
            let result = LLMStep::new("", make_request("hello"));
            assert!(result.is_err());
        }
    }

    mod chain {
        use super::*;

        #[test]
        fn builds_two_step_chain() {
            let spec = Chain::new("two-step-chain")
                .step(LLMStep::new("step1", make_request("step 1")).unwrap())
                .step(LLMStep::new("step2", make_request("step 2")).unwrap())
                .output_last("result")
                .build()
                .unwrap();

            assert_eq!(spec.kind, WorkflowKind::WorkflowV0);
            assert_eq!(spec.name.as_deref(), Some("two-step-chain"));
            assert_eq!(spec.nodes.len(), 2);
            assert_eq!(spec.edges.as_ref().map(|e| e.len()), Some(1));
            assert_eq!(spec.outputs.len(), 1);

            // Check first node has no bindings
            let node1 = &spec.nodes[0];
            assert_eq!(node1.node_type, NodeTypeV0::LlmResponses);
            let input1 = node1.input.as_ref().unwrap();
            assert!(input1.get("bindings").is_none());

            // Check second node has binding from first
            let node2 = &spec.nodes[1];
            let input2 = node2.input.as_ref().unwrap();
            assert!(input2.get("bindings").is_some());

            // Check edge
            let edges = spec.edges.as_ref().unwrap();
            assert_eq!(edges[0].from.as_str(), "step1");
            assert_eq!(edges[0].to.as_str(), "step2");

            // Check output
            assert_eq!(spec.outputs[0].name, "result");
            assert_eq!(spec.outputs[0].from.as_str(), "step2");
        }

        #[test]
        fn builds_chain_with_streaming() {
            let spec = Chain::new("three_step")
                .step(LLMStep::new("a", make_request("a")).unwrap())
                .step(LLMStep::new("b", make_request("b")).unwrap().with_stream())
                .step(LLMStep::new("c", make_request("c")).unwrap())
                .output_last("final")
                .build()
                .unwrap();

            assert_eq!(spec.nodes.len(), 3);
            assert_eq!(spec.edges.as_ref().map(|e| e.len()), Some(2));

            // Check streaming is set on middle node
            let node_b = &spec.nodes[1];
            let input_b = node_b.input.as_ref().unwrap();
            assert_eq!(input_b.get("stream"), Some(&Value::Bool(true)));

            // Other nodes should not have stream
            let node_a = &spec.nodes[0];
            let node_c = &spec.nodes[2];
            assert!(node_a.input.as_ref().unwrap().get("stream").is_none());
            assert!(node_c.input.as_ref().unwrap().get("stream").is_none());
        }

        #[test]
        fn empty_steps_returns_error() {
            let result = Chain::new("empty").build();
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("at least one step"));
        }

        #[test]
        fn supports_execution_config() {
            let spec = Chain::new("with_exec")
                .step(LLMStep::new("a", make_request("a")).unwrap())
                .execution(ExecutionV0 {
                    max_parallelism: Some(5),
                    node_timeout_ms: Some(30000),
                    run_timeout_ms: None,
                })
                .output_last("result")
                .build()
                .unwrap();

            let exec = spec.execution.unwrap();
            assert_eq!(exec.max_parallelism, Some(5));
            assert_eq!(exec.node_timeout_ms, Some(30000));
        }

        #[test]
        fn output_adds_output_from_specific_step() {
            let spec = Chain::new("multi_output")
                .step(LLMStep::new("a", make_request("a")).unwrap())
                .step(LLMStep::new("b", make_request("b")).unwrap())
                .output("from_a", "a")
                .unwrap()
                .output("from_b", "b")
                .unwrap()
                .build()
                .unwrap();

            assert_eq!(spec.outputs.len(), 2);
            // Outputs are sorted by name
            assert_eq!(spec.outputs[0].name, "from_a");
            assert_eq!(spec.outputs[0].from.as_str(), "a");
            assert_eq!(spec.outputs[1].name, "from_b");
            assert_eq!(spec.outputs[1].from.as_str(), "b");
        }
    }

    mod parallel {
        use super::*;

        #[test]
        fn builds_parallel_without_aggregation() {
            let spec = Parallel::new("parallel-only")
                .step(LLMStep::new("a", make_request("a")).unwrap())
                .step(LLMStep::new("b", make_request("b")).unwrap())
                .output("result_a", "a")
                .unwrap()
                .output("result_b", "b")
                .unwrap()
                .build()
                .unwrap();

            assert_eq!(spec.kind, WorkflowKind::WorkflowV0);
            assert_eq!(spec.name.as_deref(), Some("parallel-only"));
            assert_eq!(spec.nodes.len(), 2);
            assert!(spec.edges.is_none());
            assert_eq!(spec.outputs.len(), 2);
        }

        #[test]
        fn builds_parallel_with_aggregation() {
            let spec = Parallel::new("with-aggregate")
                .step(LLMStep::new("a", make_request("a")).unwrap())
                .step(LLMStep::new("b", make_request("b")).unwrap())
                .step(LLMStep::new("c", make_request("c")).unwrap())
                .aggregate("agg", make_request("aggregate"))
                .unwrap()
                .output("result", "agg")
                .unwrap()
                .build()
                .unwrap();

            // 3 parallel + join + aggregator = 5 nodes
            assert_eq!(spec.nodes.len(), 5);
            // 3 to join + join to agg = 4 edges
            assert_eq!(spec.edges.as_ref().map(|e| e.len()), Some(4));

            // Find the join node
            let join_node = spec.nodes.iter().find(|n| n.id.as_str() == "agg_join");
            assert!(join_node.is_some());
            assert_eq!(join_node.unwrap().node_type, NodeTypeV0::JoinAll);

            // Find the aggregator node
            let agg_node = spec.nodes.iter().find(|n| n.id.as_str() == "agg");
            assert!(agg_node.is_some());
            assert_eq!(agg_node.unwrap().node_type, NodeTypeV0::LlmResponses);
            let agg_input = agg_node.unwrap().input.as_ref().unwrap();
            assert!(agg_input.get("bindings").is_some());
        }

        #[test]
        fn aggregate_with_stream_enables_streaming() {
            let spec = Parallel::new("stream-agg")
                .step(LLMStep::new("a", make_request("a")).unwrap())
                .aggregate_with_stream("agg", make_request("aggregate"))
                .unwrap()
                .output("result", "agg")
                .unwrap()
                .build()
                .unwrap();

            let agg_node = spec.nodes.iter().find(|n| n.id.as_str() == "agg").unwrap();
            let agg_input = agg_node.input.as_ref().unwrap();
            assert_eq!(agg_input.get("stream"), Some(&Value::Bool(true)));
        }

        #[test]
        fn empty_steps_returns_error() {
            let result = Parallel::new("empty").build();
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("at least one step"));
        }

        #[test]
        fn supports_execution_config() {
            let spec = Parallel::new("with-exec")
                .step(LLMStep::new("a", make_request("a")).unwrap())
                .execution(ExecutionV0 {
                    max_parallelism: Some(10),
                    node_timeout_ms: None,
                    run_timeout_ms: None,
                })
                .output("result", "a")
                .unwrap()
                .build()
                .unwrap();

            assert_eq!(spec.execution.unwrap().max_parallelism, Some(10));
        }
    }

    mod map_item {
        use super::*;

        #[test]
        fn creates_map_item() {
            let item = MapItem::new("item1", make_request("process")).unwrap();
            assert_eq!(item.id(), "item1");
            assert!(!item.stream);
        }

        #[test]
        fn with_stream_enables_streaming() {
            let item = MapItem::new("item1", make_request("process"))
                .unwrap()
                .with_stream();
            assert!(item.stream);
        }

        #[test]
        fn empty_id_returns_error() {
            let result = MapItem::new("", make_request("process"));
            assert!(result.is_err());
        }
    }

    mod map_reduce {
        use super::*;

        #[test]
        fn builds_three_item_map_reduce() {
            let spec = MapReduce::new("three-items")
                .item(MapItem::new("a", make_request("process a")).unwrap())
                .item(MapItem::new("b", make_request("process b")).unwrap())
                .item(MapItem::new("c", make_request("process c")).unwrap())
                .reduce("reducer", make_request("combine"))
                .unwrap()
                .output("result", "reducer")
                .unwrap()
                .build()
                .unwrap();

            assert_eq!(spec.kind, WorkflowKind::WorkflowV0);
            assert_eq!(spec.name.as_deref(), Some("three-items"));
            // 3 mappers + join + reducer = 5 nodes
            assert_eq!(spec.nodes.len(), 5);
            // 3 to join + join to reducer = 4 edges
            assert_eq!(spec.edges.as_ref().map(|e| e.len()), Some(4));

            // Check mapper nodes exist with correct IDs
            assert!(spec.nodes.iter().any(|n| n.id.as_str() == "map_a"));
            assert!(spec.nodes.iter().any(|n| n.id.as_str() == "map_b"));
            assert!(spec.nodes.iter().any(|n| n.id.as_str() == "map_c"));

            // Check join node
            let join_node = spec.nodes.iter().find(|n| n.id.as_str() == "reducer_join");
            assert!(join_node.is_some());
            assert_eq!(join_node.unwrap().node_type, NodeTypeV0::JoinAll);

            // Check reducer node
            let reducer_node = spec.nodes.iter().find(|n| n.id.as_str() == "reducer");
            assert!(reducer_node.is_some());
            let reducer_input = reducer_node.unwrap().input.as_ref().unwrap();
            assert!(reducer_input.get("bindings").is_some());

            // Check output
            assert_eq!(spec.outputs[0].name, "result");
            assert_eq!(spec.outputs[0].from.as_str(), "reducer");
        }

        #[test]
        fn builds_with_streaming() {
            let spec = MapReduce::new("streaming")
                .item(MapItem::new("a", make_request("a")).unwrap().with_stream())
                .item(MapItem::new("b", make_request("b")).unwrap())
                .reduce_with_stream("reducer", make_request("combine"))
                .unwrap()
                .output("result", "reducer")
                .unwrap()
                .build()
                .unwrap();

            // Check streaming on mapper a
            let mapper_a = spec
                .nodes
                .iter()
                .find(|n| n.id.as_str() == "map_a")
                .unwrap();
            let input_a = mapper_a.input.as_ref().unwrap();
            assert_eq!(input_a.get("stream"), Some(&Value::Bool(true)));

            // Check mapper b has no streaming
            let mapper_b = spec
                .nodes
                .iter()
                .find(|n| n.id.as_str() == "map_b")
                .unwrap();
            let input_b = mapper_b.input.as_ref().unwrap();
            assert!(input_b.get("stream").is_none());

            // Check streaming on reducer
            let reducer = spec
                .nodes
                .iter()
                .find(|n| n.id.as_str() == "reducer")
                .unwrap();
            let reducer_input = reducer.input.as_ref().unwrap();
            assert_eq!(reducer_input.get("stream"), Some(&Value::Bool(true)));
        }

        #[test]
        fn empty_items_returns_error() {
            let result = MapReduce::new("empty")
                .reduce("r", make_request("r"))
                .unwrap()
                .build();
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("at least one item"));
        }

        #[test]
        fn no_reducer_returns_error() {
            let result = MapReduce::new("no-reducer")
                .item(MapItem::new("a", make_request("a")).unwrap())
                .build();
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("reducer"));
        }

        #[test]
        fn duplicate_item_ids_returns_error() {
            let result = MapReduce::new("dup")
                .item(MapItem::new("same", make_request("a")).unwrap())
                .item(MapItem::new("same", make_request("b")).unwrap())
                .reduce("r", make_request("r"))
                .unwrap()
                .build();
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("duplicate"));
        }

        #[test]
        fn supports_execution_config() {
            let spec = MapReduce::new("with-exec")
                .item(MapItem::new("a", make_request("a")).unwrap())
                .execution(ExecutionV0 {
                    max_parallelism: Some(8),
                    node_timeout_ms: None,
                    run_timeout_ms: Some(60000),
                })
                .reduce("r", make_request("r"))
                .unwrap()
                .output("result", "r")
                .unwrap()
                .build()
                .unwrap();

            let exec = spec.execution.unwrap();
            assert_eq!(exec.max_parallelism, Some(8));
            assert_eq!(exec.run_timeout_ms, Some(60000));
        }

        #[test]
        fn sorts_edges_deterministically() {
            // Create items in non-alphabetical order
            let spec = MapReduce::new("sorted")
                .item(MapItem::new("c", make_request("c")).unwrap())
                .item(MapItem::new("a", make_request("a")).unwrap())
                .item(MapItem::new("b", make_request("b")).unwrap())
                .reduce("r", make_request("r"))
                .unwrap()
                .output("result", "r")
                .unwrap()
                .build()
                .unwrap();

            // Edges should be sorted by from, then to
            let edges = spec.edges.as_ref().unwrap();
            assert_eq!(edges[0].from.as_str(), "map_a");
            assert_eq!(edges[1].from.as_str(), "map_b");
            assert_eq!(edges[2].from.as_str(), "map_c");
            assert_eq!(edges[3].from.as_str(), "r_join");
        }
    }
}
