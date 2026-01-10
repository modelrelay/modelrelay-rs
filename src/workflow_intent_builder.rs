use std::collections::BTreeMap;

use crate::errors::{Error, Result, ValidationError};
use crate::types::InputItem;
use crate::workflow_intent::{
    WorkflowIntentCondition, WorkflowIntentKind, WorkflowIntentNode, WorkflowIntentNodeType,
    WorkflowIntentOutputRef, WorkflowIntentSpec, WorkflowIntentToolExecution,
    WorkflowIntentToolExecutionMode, WorkflowIntentToolRef, WorkflowIntentTransformValue,
};

#[derive(Debug, Clone)]
struct WorkflowIntentEdge {
    from: String,
    to: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowIntentBuilder {
    name: Option<String>,
    model: Option<String>,
    nodes: Vec<WorkflowIntentNode>,
    edges: Vec<WorkflowIntentEdge>,
    outputs: Vec<WorkflowIntentOutputRef>,
}

#[derive(Debug, Clone, Default)]
pub struct JoinCollectOptions {
    pub limit: Option<i64>,
    pub timeout_ms: Option<i64>,
    pub predicate: Option<WorkflowIntentCondition>,
}

#[derive(Debug, Clone)]
pub struct MapFanoutOptions {
    pub items_from: Option<String>,
    pub items_from_input: Option<String>,
    pub items_path: Option<String>,
    pub subnode: WorkflowIntentNode,
    pub max_parallelism: Option<i64>,
}

impl WorkflowIntentBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into().trim().to_string());
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into().trim().to_string());
        self
    }

    pub fn node(mut self, node: WorkflowIntentNode) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn llm<F>(self, id: impl Into<String>, configure: F) -> Self
    where
        F: FnOnce(LLMNodeBuilder) -> LLMNodeBuilder,
    {
        let builder = LLMNodeBuilder::new(id);
        let node = configure(builder).build();
        self.node(node)
    }

    pub fn join_all(self, id: impl Into<String>) -> Self {
        self.node(WorkflowIntentNode::with_type(
            id,
            WorkflowIntentNodeType::JoinAll,
        ))
    }

    pub fn join_any(
        self,
        id: impl Into<String>,
        predicate: Option<WorkflowIntentCondition>,
    ) -> Self {
        let mut node = WorkflowIntentNode::with_type(id, WorkflowIntentNodeType::JoinAny);
        node.predicate = predicate;
        self.node(node)
    }

    pub fn join_collect(self, id: impl Into<String>, options: JoinCollectOptions) -> Self {
        let mut node = WorkflowIntentNode::with_type(id, WorkflowIntentNodeType::JoinCollect);
        node.limit = options.limit;
        node.timeout_ms = options.timeout_ms;
        node.predicate = options.predicate;
        self.node(node)
    }

    pub fn transform_json(
        self,
        id: impl Into<String>,
        object: Option<BTreeMap<String, WorkflowIntentTransformValue>>,
        merge: Option<Vec<WorkflowIntentTransformValue>>,
    ) -> Self {
        let mut node = WorkflowIntentNode::with_type(id, WorkflowIntentNodeType::TransformJson);
        node.object = object;
        node.merge = merge;
        self.node(node)
    }

    pub fn map_fanout(self, id: impl Into<String>, options: MapFanoutOptions) -> Self {
        let mut node = WorkflowIntentNode::with_type(id, WorkflowIntentNodeType::MapFanout);
        node.items_from = options.items_from;
        node.items_from_input = options.items_from_input;
        node.items_path = options.items_path;
        node.subnode = Some(Box::new(options.subnode));
        node.max_parallelism = options.max_parallelism;
        self.node(node)
    }

    pub fn edge(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.edges.push(WorkflowIntentEdge {
            from: from.into(),
            to: to.into(),
        });
        self
    }

    pub fn output(
        mut self,
        name: impl Into<String>,
        from: impl Into<String>,
        pointer: Option<String>,
    ) -> Self {
        self.outputs.push(WorkflowIntentOutputRef {
            name: name.into(),
            from: from.into(),
            pointer,
        });
        self
    }

    pub fn build(self) -> Result<WorkflowIntentSpec> {
        let mut nodes = self.nodes;
        let mut index = std::collections::HashMap::new();
        for (idx, node) in nodes.iter().enumerate() {
            index.insert(node.id.clone(), idx);
        }

        for edge in self.edges {
            let idx = match index.get(&edge.to) {
                Some(i) => *i,
                None => {
                    return Err(Error::Validation(ValidationError::new(format!(
                        "edge to unknown node {}",
                        edge.to
                    ))))
                }
            };
            let depends = nodes[idx].depends_on.get_or_insert_with(Vec::new);
            if !depends.contains(&edge.from) {
                depends.push(edge.from);
            }
        }

        Ok(WorkflowIntentSpec {
            kind: WorkflowIntentKind::WorkflowIntent,
            name: self.name,
            model: self.model,
            nodes,
            outputs: self.outputs,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LLMNodeBuilder {
    node: WorkflowIntentNode,
}

impl LLMNodeBuilder {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            node: WorkflowIntentNode::with_type(id, WorkflowIntentNodeType::Llm),
        }
    }

    pub fn system(mut self, text: impl Into<String>) -> Self {
        self.node.system = Some(text.into());
        self
    }

    pub fn user(mut self, text: impl Into<String>) -> Self {
        self.node.user = Some(text.into());
        self
    }

    pub fn input(mut self, items: Vec<InputItem>) -> Self {
        self.node.input = Some(items);
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.node.model = Some(model.into());
        self
    }

    pub fn stream(mut self, enabled: bool) -> Self {
        self.node.stream = Some(enabled);
        self
    }

    pub fn tool_execution(mut self, mode: WorkflowIntentToolExecutionMode) -> Self {
        self.node.tool_execution = Some(WorkflowIntentToolExecution { mode });
        self
    }

    pub fn tools<I, T>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<WorkflowIntentToolRef>,
    {
        let refs: Vec<WorkflowIntentToolRef> = tools.into_iter().map(Into::into).collect();
        self.node.tools = Some(refs);
        self
    }

    pub fn build(self) -> WorkflowIntentNode {
        self.node
    }
}

pub fn workflow_intent() -> WorkflowIntentBuilder {
    WorkflowIntentBuilder::new()
}

/// Alias for workflow_intent() with a cleaner name.
pub fn workflow() -> WorkflowIntentBuilder {
    WorkflowIntentBuilder::new()
}

/// Creates a standalone LLM node for use with chain() and parallel().
pub fn llm<F>(id: impl Into<String>, configure: F) -> WorkflowIntentNode
where
    F: FnOnce(LLMNodeBuilder) -> LLMNodeBuilder,
{
    let builder = LLMNodeBuilder::new(id);
    configure(builder).build()
}

/// Options for chain() helper.
#[derive(Debug, Clone, Default)]
pub struct ChainOptions {
    pub name: Option<String>,
    pub model: Option<String>,
}

/// Creates a sequential workflow where each step depends on the previous one.
/// Edges are automatically wired based on order.
///
/// # Example
/// ```ignore
/// let spec = chain(
///     vec![
///         llm("summarize", |n| n.system("Summarize.").user("{{task}}")),
///         llm("translate", |n| n.system("Translate to French.").user("{{summarize}}")),
///     ],
///     ChainOptions { name: Some("summarize-translate".into()), ..Default::default() },
/// )
/// .output("result", "translate", None)
/// .build()?;
/// ```
pub fn chain(steps: Vec<WorkflowIntentNode>, options: ChainOptions) -> WorkflowIntentBuilder {
    let mut builder = WorkflowIntentBuilder::new();

    if let Some(name) = options.name {
        builder = builder.name(name);
    }
    if let Some(model) = options.model {
        builder = builder.model(model);
    }

    // Add all nodes
    for step in &steps {
        builder = builder.node(step.clone());
    }

    // Wire edges sequentially: step[0] -> step[1] -> step[2] -> ...
    for i in 1..steps.len() {
        builder = builder.edge(&steps[i - 1].id, &steps[i].id);
    }

    builder
}

/// Options for parallel() helper.
#[derive(Debug, Clone, Default)]
pub struct ParallelOptions {
    pub name: Option<String>,
    pub model: Option<String>,
    /// ID for the join node (default: "join")
    pub join_id: Option<String>,
}

/// Creates a parallel workflow where all steps run concurrently, then join.
/// Edges are automatically wired to a join.all node.
///
/// # Example
/// ```ignore
/// let spec = parallel(
///     vec![
///         llm("agent_a", |n| n.user("Write 3 ideas for {{task}}")),
///         llm("agent_b", |n| n.user("Write 3 objections for {{task}}")),
///     ],
///     ParallelOptions { name: Some("multi-agent".into()), ..Default::default() },
/// )
/// .llm("aggregate", |n| n.system("Synthesize.").user("{{join}}"))
/// .edge("join", "aggregate")
/// .output("result", "aggregate", None)
/// .build()?;
/// ```
pub fn parallel(steps: Vec<WorkflowIntentNode>, options: ParallelOptions) -> WorkflowIntentBuilder {
    let mut builder = WorkflowIntentBuilder::new();
    let join_id = options.join_id.unwrap_or_else(|| "join".to_string());

    if let Some(name) = options.name {
        builder = builder.name(name);
    }
    if let Some(model) = options.model {
        builder = builder.model(model);
    }

    // Add all parallel nodes
    for step in &steps {
        builder = builder.node(step.clone());
    }

    // Add join node
    builder = builder.join_all(&join_id);

    // Wire all parallel nodes to the join
    for step in &steps {
        builder = builder.edge(&step.id, &join_id);
    }

    builder
}
