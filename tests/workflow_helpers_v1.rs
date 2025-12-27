//! Tests for workflow.v1 condition and binding helper functions.

use modelrelay::{
    bind_to_placeholder, bind_to_placeholder_with_pointer, bind_to_pointer,
    bind_to_pointer_with_source, when_output_equals, when_output_exists, when_output_matches,
    when_status_equals, when_status_exists, when_status_matches, BindingBuilder, ConditionOpV1,
    ConditionSourceV1, LlmResponsesBindingEncodingV1, NodeId,
};

/// Helper to parse a node ID (panics on invalid input).
fn node(s: &str) -> NodeId {
    s.parse().expect("valid node id")
}

// =============================================================================
// Condition Factory Tests
// =============================================================================

#[test]
fn when_output_equals_creates_correct_condition() {
    let cond = when_output_equals("$.route", "billing");

    assert_eq!(cond.source, ConditionSourceV1::NodeOutput);
    assert_eq!(cond.op, ConditionOpV1::Equals);
    assert_eq!(cond.path, "$.route");
    assert_eq!(cond.value, Some(serde_json::json!("billing")));
}

#[test]
fn when_output_equals_with_numeric_value() {
    let cond = when_output_equals("$.count", 42);

    assert_eq!(cond.source, ConditionSourceV1::NodeOutput);
    assert_eq!(cond.op, ConditionOpV1::Equals);
    assert_eq!(cond.path, "$.count");
    assert_eq!(cond.value, Some(serde_json::json!(42)));
}

#[test]
fn when_output_matches_creates_correct_condition() {
    let cond = when_output_matches("$.category", "billing|support");

    assert_eq!(cond.source, ConditionSourceV1::NodeOutput);
    assert_eq!(cond.op, ConditionOpV1::Matches);
    assert_eq!(cond.path, "$.category");
    assert_eq!(cond.value, Some(serde_json::json!("billing|support")));
}

#[test]
fn when_output_exists_creates_correct_condition() {
    let cond = when_output_exists("$.special_case");

    assert_eq!(cond.source, ConditionSourceV1::NodeOutput);
    assert_eq!(cond.op, ConditionOpV1::Exists);
    assert_eq!(cond.path, "$.special_case");
    assert_eq!(cond.value, None);
}

#[test]
fn when_status_equals_creates_correct_condition() {
    let cond = when_status_equals("$.state", "completed");

    assert_eq!(cond.source, ConditionSourceV1::NodeStatus);
    assert_eq!(cond.op, ConditionOpV1::Equals);
    assert_eq!(cond.path, "$.state");
    assert_eq!(cond.value, Some(serde_json::json!("completed")));
}

#[test]
fn when_status_matches_creates_correct_condition() {
    let cond = when_status_matches("$.state", "success|complete");

    assert_eq!(cond.source, ConditionSourceV1::NodeStatus);
    assert_eq!(cond.op, ConditionOpV1::Matches);
    assert_eq!(cond.path, "$.state");
    assert_eq!(cond.value, Some(serde_json::json!("success|complete")));
}

#[test]
fn when_status_exists_creates_correct_condition() {
    let cond = when_status_exists("$.error");

    assert_eq!(cond.source, ConditionSourceV1::NodeStatus);
    assert_eq!(cond.op, ConditionOpV1::Exists);
    assert_eq!(cond.path, "$.error");
    assert_eq!(cond.value, None);
}

// =============================================================================
// Binding Factory Tests
// =============================================================================

#[test]
fn bind_to_placeholder_creates_correct_binding() {
    let binding = bind_to_placeholder(node("router"), "route_data");

    assert_eq!(binding.from.to_string(), "router");
    assert_eq!(binding.pointer, None);
    assert_eq!(binding.to, None);
    assert_eq!(binding.to_placeholder, Some("route_data".to_string()));
    assert_eq!(
        binding.encoding,
        Some(LlmResponsesBindingEncodingV1::JsonString)
    );
}

#[test]
fn bind_to_placeholder_with_pointer_creates_correct_binding() {
    let binding = bind_to_placeholder_with_pointer(node("fanout"), "/results", "data");

    assert_eq!(binding.from.to_string(), "fanout");
    assert_eq!(binding.pointer, Some("/results".to_string()));
    assert_eq!(binding.to, None);
    assert_eq!(binding.to_placeholder, Some("data".to_string()));
    assert_eq!(
        binding.encoding,
        Some(LlmResponsesBindingEncodingV1::JsonString)
    );
}

#[test]
fn bind_to_pointer_creates_correct_binding() {
    let binding = bind_to_pointer(node("source"), "/input/0/content/0/text");

    assert_eq!(binding.from.to_string(), "source");
    assert_eq!(binding.pointer, None);
    assert_eq!(binding.to, Some("/input/0/content/0/text".to_string()));
    assert_eq!(binding.to_placeholder, None);
    assert_eq!(
        binding.encoding,
        Some(LlmResponsesBindingEncodingV1::JsonString)
    );
}

#[test]
fn bind_to_pointer_with_source_creates_correct_binding() {
    let binding =
        bind_to_pointer_with_source(node("source"), "/output/text", "/input/0/content/0/text");

    assert_eq!(binding.from.to_string(), "source");
    assert_eq!(binding.pointer, Some("/output/text".to_string()));
    assert_eq!(binding.to, Some("/input/0/content/0/text".to_string()));
    assert_eq!(binding.to_placeholder, None);
    assert_eq!(
        binding.encoding,
        Some(LlmResponsesBindingEncodingV1::JsonString)
    );
}

// =============================================================================
// BindingBuilder Tests
// =============================================================================

#[test]
fn binding_builder_to_placeholder() {
    let binding = BindingBuilder::from(node("source"))
        .pointer("/output/text")
        .to_placeholder("data")
        .build();

    assert_eq!(binding.from.to_string(), "source");
    assert_eq!(binding.pointer, Some("/output/text".to_string()));
    assert_eq!(binding.to, None);
    assert_eq!(binding.to_placeholder, Some("data".to_string()));
    assert_eq!(
        binding.encoding,
        Some(LlmResponsesBindingEncodingV1::JsonString)
    );
}

#[test]
fn binding_builder_to_pointer() {
    let binding = BindingBuilder::from(node("source"))
        .pointer("/output/text")
        .to("/input/0/content/0/text")
        .build();

    assert_eq!(binding.from.to_string(), "source");
    assert_eq!(binding.pointer, Some("/output/text".to_string()));
    assert_eq!(binding.to, Some("/input/0/content/0/text".to_string()));
    assert_eq!(binding.to_placeholder, None);
}

#[test]
fn binding_builder_with_custom_encoding() {
    let binding = BindingBuilder::from(node("source"))
        .to_placeholder("data")
        .encoding(LlmResponsesBindingEncodingV1::Json)
        .build();

    assert_eq!(binding.encoding, Some(LlmResponsesBindingEncodingV1::Json));
}

#[test]
fn binding_builder_to_overrides_to_placeholder() {
    // Setting .to() should clear to_placeholder
    let binding = BindingBuilder::from(node("source"))
        .to_placeholder("data")
        .to("/input/0/content/0/text")
        .build();

    assert_eq!(binding.to, Some("/input/0/content/0/text".to_string()));
    assert_eq!(binding.to_placeholder, None);
}

#[test]
fn binding_builder_to_placeholder_overrides_to() {
    // Setting .to_placeholder() should clear to
    let binding = BindingBuilder::from(node("source"))
        .to("/input/0/content/0/text")
        .to_placeholder("data")
        .build();

    assert_eq!(binding.to, None);
    assert_eq!(binding.to_placeholder, Some("data".to_string()));
}
