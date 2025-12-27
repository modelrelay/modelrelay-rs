//! Helper functions for constructing workflow.v1 conditions and bindings.
//!
//! These factory functions reduce boilerplate when building conditional edges
//! and node bindings.
//!
//! # Example
//!
//! ```ignore
//! use modelrelay::workflow::helpers_v1::*;
//! use modelrelay::workflow_builder::WorkflowBuilderV1;
//!
//! let spec = WorkflowBuilderV1::new()
//!     .route_switch("router", request)?
//!     .llm_responses_with_bindings("billing", billing_req, vec![
//!         bind_to_placeholder("router", "route_data")
//!     ])?
//!     .edge_when("router", "billing", when_output_equals("$.route", "billing"))
//!     .build();
//! ```

use serde_json::Value;

use super::spec::{ConditionOpV1, ConditionSourceV1, ConditionV1};
use super::NodeId;
use crate::LlmResponsesBindingEncodingV1;
use crate::LlmResponsesBindingV1;

// =============================================================================
// Condition Factories
// =============================================================================

/// Create a condition that matches when a node's output equals a specific value.
///
/// # Arguments
///
/// * `path` - JSONPath expression to extract the value (must start with $)
/// * `value` - The value to compare against
///
/// # Example
///
/// ```ignore
/// builder.edge_when("router", "billing", when_output_equals("$.route", "billing"))
/// ```
pub fn when_output_equals<V: Into<Value>>(path: &str, value: V) -> ConditionV1 {
    ConditionV1 {
        source: ConditionSourceV1::NodeOutput,
        op: ConditionOpV1::Equals,
        path: path.to_string(),
        value: Some(value.into()),
    }
}

/// Create a condition that matches when a node's output matches a regex pattern.
///
/// # Arguments
///
/// * `path` - JSONPath expression to extract the value (must start with $)
/// * `pattern` - Regular expression pattern to match
///
/// # Example
///
/// ```ignore
/// builder.edge_when("router", "handler", when_output_matches("$.category", "billing|support"))
/// ```
pub fn when_output_matches(path: &str, pattern: &str) -> ConditionV1 {
    ConditionV1 {
        source: ConditionSourceV1::NodeOutput,
        op: ConditionOpV1::Matches,
        path: path.to_string(),
        value: Some(Value::String(pattern.to_string())),
    }
}

/// Create a condition that matches when a path exists in the node's output.
///
/// # Arguments
///
/// * `path` - JSONPath expression to check for existence (must start with $)
///
/// # Example
///
/// ```ignore
/// builder.edge_when("router", "handler", when_output_exists("$.special_case"))
/// ```
pub fn when_output_exists(path: &str) -> ConditionV1 {
    ConditionV1 {
        source: ConditionSourceV1::NodeOutput,
        op: ConditionOpV1::Exists,
        path: path.to_string(),
        value: None,
    }
}

/// Create a condition that matches when a node's status equals a specific value.
///
/// # Arguments
///
/// * `path` - JSONPath expression to extract the status value (must start with $)
/// * `value` - The status value to compare against
pub fn when_status_equals<V: Into<Value>>(path: &str, value: V) -> ConditionV1 {
    ConditionV1 {
        source: ConditionSourceV1::NodeStatus,
        op: ConditionOpV1::Equals,
        path: path.to_string(),
        value: Some(value.into()),
    }
}

/// Create a condition that matches when a node's status matches a regex pattern.
pub fn when_status_matches(path: &str, pattern: &str) -> ConditionV1 {
    ConditionV1 {
        source: ConditionSourceV1::NodeStatus,
        op: ConditionOpV1::Matches,
        path: path.to_string(),
        value: Some(Value::String(pattern.to_string())),
    }
}

/// Create a condition that matches when a path exists in the node's status.
pub fn when_status_exists(path: &str) -> ConditionV1 {
    ConditionV1 {
        source: ConditionSourceV1::NodeStatus,
        op: ConditionOpV1::Exists,
        path: path.to_string(),
        value: None,
    }
}

// =============================================================================
// Binding Factories
// =============================================================================

/// Create a binding that injects a value into a {{placeholder}} in the prompt.
/// Uses json_string encoding by default.
///
/// # Arguments
///
/// * `from` - Source node ID
/// * `placeholder` - Placeholder name (without the {{ }} delimiters)
///
/// # Example
///
/// ```ignore
/// builder.llm_responses_with_bindings("aggregate", request, vec![
///     bind_to_placeholder("join", "route_output"),
/// ])
/// ```
pub fn bind_to_placeholder(from: impl Into<NodeId>, placeholder: &str) -> LlmResponsesBindingV1 {
    LlmResponsesBindingV1 {
        from: from.into(),
        pointer: None,
        to: None,
        to_placeholder: Some(placeholder.to_string()),
        encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
    }
}

/// Create a binding with a source pointer that injects into a placeholder.
///
/// # Arguments
///
/// * `from` - Source node ID
/// * `pointer` - JSON pointer to extract from the source node's output
/// * `placeholder` - Placeholder name (without the {{ }} delimiters)
///
/// # Example
///
/// ```ignore
/// builder.llm_responses_with_bindings("aggregate", request, vec![
///     bind_to_placeholder_with_pointer("fanout", "/results", "data"),
/// ])
/// ```
pub fn bind_to_placeholder_with_pointer(
    from: impl Into<NodeId>,
    pointer: &str,
    placeholder: &str,
) -> LlmResponsesBindingV1 {
    LlmResponsesBindingV1 {
        from: from.into(),
        pointer: Some(pointer.to_string()),
        to: None,
        to_placeholder: Some(placeholder.to_string()),
        encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
    }
}

/// Create a binding that injects a value at a specific JSON pointer in the request.
/// Uses json_string encoding by default.
///
/// # Arguments
///
/// * `from` - Source node ID
/// * `to` - JSON pointer in the request to inject the value
///
/// # Example
///
/// ```ignore
/// builder.llm_responses_with_bindings("processor", request, vec![
///     bind_to_pointer("source", "/input/0/content/0/text"),
/// ])
/// ```
pub fn bind_to_pointer(from: impl Into<NodeId>, to: &str) -> LlmResponsesBindingV1 {
    LlmResponsesBindingV1 {
        from: from.into(),
        pointer: None,
        to: Some(to.to_string()),
        to_placeholder: None,
        encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
    }
}

/// Create a binding with both source and destination pointers.
///
/// # Arguments
///
/// * `from` - Source node ID
/// * `source_pointer` - JSON pointer to extract from the source node's output
/// * `to` - JSON pointer in the request to inject the value
pub fn bind_to_pointer_with_source(
    from: impl Into<NodeId>,
    source_pointer: &str,
    to: &str,
) -> LlmResponsesBindingV1 {
    LlmResponsesBindingV1 {
        from: from.into(),
        pointer: Some(source_pointer.to_string()),
        to: Some(to.to_string()),
        to_placeholder: None,
        encoding: Some(LlmResponsesBindingEncodingV1::JsonString),
    }
}

/// Fluent builder for constructing bindings.
///
/// # Example
///
/// ```ignore
/// let binding = BindingBuilder::from("source")
///     .pointer("/output/text")
///     .to_placeholder("data")
///     .build();
/// ```
pub struct BindingBuilder {
    from: NodeId,
    pointer: Option<String>,
    to: Option<String>,
    to_placeholder: Option<String>,
    encoding: LlmResponsesBindingEncodingV1,
}

impl BindingBuilder {
    /// Create a new BindingBuilder starting with the source node.
    pub fn from(from: impl Into<NodeId>) -> Self {
        Self {
            from: from.into(),
            pointer: None,
            to: None,
            to_placeholder: None,
            encoding: LlmResponsesBindingEncodingV1::JsonString,
        }
    }

    /// Set the source pointer to extract from the node's output.
    pub fn pointer(mut self, ptr: &str) -> Self {
        self.pointer = Some(ptr.to_string());
        self
    }

    /// Set the destination JSON pointer in the request.
    pub fn to(mut self, ptr: &str) -> Self {
        self.to = Some(ptr.to_string());
        self.to_placeholder = None;
        self
    }

    /// Set the destination placeholder name.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_placeholder(mut self, name: &str) -> Self {
        self.to_placeholder = Some(name.to_string());
        self.to = None;
        self
    }

    /// Set the encoding for the binding value.
    pub fn encoding(mut self, enc: LlmResponsesBindingEncodingV1) -> Self {
        self.encoding = enc;
        self
    }

    /// Build the binding.
    pub fn build(self) -> LlmResponsesBindingV1 {
        LlmResponsesBindingV1 {
            from: self.from,
            pointer: self.pointer,
            to: self.to,
            to_placeholder: self.to_placeholder,
            encoding: Some(self.encoding),
        }
    }
}
