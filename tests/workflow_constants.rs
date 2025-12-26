//! Tests verifying SDK JSON pointer constants match platform canonical definitions.
//!
//! This would have caught the "/request/input/..." vs "/input/..." bug that caused
//! production failures.
//!
//! Platform canonical definitions (source of truth in platform/workflow/constants.go):
//! - LLMTextOutputPointer = "/output/0/content/0/text"
//! - LLMUserMessageTextPointer = "/input/0/content/0/text" (index 0, no system message)
//! - LLMUserMessageTextPointerIndex1 = "/input/1/content/0/text" (index 1, with system message)

use modelrelay::{LLM_TEXT_OUTPUT, LLM_USER_MESSAGE_TEXT};

// Platform canonical values (from platform/workflow/constants.go)
const PLATFORM_LLM_TEXT_OUTPUT: &str = "/output/0/content/0/text";
const PLATFORM_LLM_USER_MESSAGE_TEXT_INDEX1: &str = "/input/1/content/0/text";

#[test]
fn llm_text_output_matches_platform() {
    assert_eq!(
        LLM_TEXT_OUTPUT, PLATFORM_LLM_TEXT_OUTPUT,
        "LLM_TEXT_OUTPUT must match platform canonical definition"
    );
}

#[test]
fn llm_text_output_starts_with_output() {
    assert!(
        LLM_TEXT_OUTPUT.starts_with("/output"),
        "LLM_TEXT_OUTPUT should start with /output (extraction from response)"
    );
}

#[test]
fn llm_user_message_text_matches_platform() {
    // SDK uses index 1 because ResponseBuilder.system() puts system message at index 0
    assert_eq!(
        LLM_USER_MESSAGE_TEXT, PLATFORM_LLM_USER_MESSAGE_TEXT_INDEX1,
        "LLM_USER_MESSAGE_TEXT must match platform canonical definition (index 1)"
    );
}

#[test]
fn llm_user_message_text_no_request_prefix() {
    // This test catches the exact bug we fixed: binding targets are relative
    // to the request object, not the full node input
    assert!(
        !LLM_USER_MESSAGE_TEXT.starts_with("/request/"),
        "LLM_USER_MESSAGE_TEXT must NOT have /request/ prefix - binding targets are relative to request object"
    );
}

#[test]
fn llm_user_message_text_starts_with_input() {
    assert!(
        LLM_USER_MESSAGE_TEXT.starts_with("/input"),
        "LLM_USER_MESSAGE_TEXT should start with /input (injection into request)"
    );
}

#[test]
fn llm_text_output_valid_rfc6901_format() {
    // Must start with /
    assert!(
        LLM_TEXT_OUTPUT.starts_with('/'),
        "JSON pointer must start with /"
    );
    // Must not have empty segments (no //)
    assert!(
        !LLM_TEXT_OUTPUT.contains("//"),
        "JSON pointer must not have empty segments"
    );
}

#[test]
fn llm_user_message_text_valid_rfc6901_format() {
    assert!(
        LLM_USER_MESSAGE_TEXT.starts_with('/'),
        "JSON pointer must start with /"
    );
    assert!(
        !LLM_USER_MESSAGE_TEXT.contains("//"),
        "JSON pointer must not have empty segments"
    );
}
