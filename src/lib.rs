//! Minimal Rust SDK for the ModelRelay API.
#![cfg_attr(docsrs, feature(doc_cfg))]
// Allow large error types - refactoring to Box<Error> would be a breaking change
#![allow(clippy::result_large_err)]

/// Generated types from OpenAPI spec.
/// Run `just generate-sdk-types` to regenerate from api/openapi/api.yaml.
pub mod generated;

/// Default API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.modelrelay.ai/api/v1";

/// Default User-Agent header value.
pub(crate) const DEFAULT_CLIENT_HEADER: &str =
    concat!("modelrelay-rust/", env!("CARGO_PKG_VERSION"));

/// Default connection timeout (5 seconds).
pub const DEFAULT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Default request timeout (60 seconds).
pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// HTTP header name for request ID tracing.
pub const REQUEST_ID_HEADER: &str = "X-ModelRelay-Request-Id";

/// HTTP header name for API key authentication.
pub(crate) const API_KEY_HEADER: &str = "X-ModelRelay-Api-Key";

mod api_key;
mod bash_policy;
mod bash_tokenizer;
#[cfg(feature = "billing")]
mod billing;
mod client;
mod core;
mod errors;
mod http;
mod identifiers;
mod images;
mod local_bash_tools;
mod local_fs_tools;
mod local_tools_common;
#[cfg(feature = "mock")]
mod mock;
mod responses;
mod runs;
mod sessions;
mod structured;
mod telemetry;
pub mod testing;
mod tiers;
mod token_providers;
pub mod tools;
mod types;
mod workflow;
mod workflow_intent;
mod workflow_intent_builder;
mod workflows;

// Re-export common types used in public API for user convenience
pub use chrono::{DateTime, Utc};
pub use uuid::Uuid;

pub use api_key::{ApiKey, SecretKey};
#[cfg(feature = "billing")]
pub use billing::BillingClient;
pub use errors::{
    APIError, Error, FieldError, RetryMetadata, StreamTimeoutError, StreamTimeoutKind,
    TransportError, TransportErrorKind, ValidationError, WorkflowValidationError,
    WorkflowValidationIssue,
};
pub use http::{HeaderEntry, HeaderList, ResponseOptions, RetryConfig};
#[cfg(all(feature = "mock", feature = "blocking"))]
pub use mock::MockBlockingResponsesClient;
#[cfg(feature = "mock")]
pub use mock::{fixtures, MockAuthClient, MockClient, MockConfig, MockResponsesClient};
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use responses::BlockingStructuredJSONStream;
#[cfg(feature = "streaming")]
pub use responses::StructuredJSONStream;
pub use responses::{ResponseBuilder, ResponseStreamAdapter, CUSTOMER_ID_HEADER};
pub use responses::{StructuredJSONEvent, StructuredRecordKind};
pub use telemetry::{
    HttpRequestMetrics, MetricsCallbacks, RequestContext, StreamFirstTokenMetrics,
    TokenUsageMetrics,
};
pub use tools::{
    assistant_message_with_tool_calls, create_retry_messages, execute_with_retry,
    format_tool_error_for_model, get_retryable_errors, has_retryable_errors,
    parse_and_validate_tool_args, parse_tool_args, respond_to_tool_call, respond_to_tool_call_json,
    sync_handler, tool_result_message, tool_result_message_json, BoxFuture, ParseResult,
    ResponseExt, RetryOptions, ToolArgsError, ToolCallAccumulator, ToolExecutionResult,
    ToolHandler, ToolRegistry, UnknownToolError, ValidateArgs,
};
pub use tools::{function_tool_from_type, ToolSchema};
pub use types::{
    APIKey, Citation, CodeExecConfig, ContentPart, CustomerToken, CustomerTokenRequest,
    FunctionCall, FunctionCallDelta, FunctionTool, GetOrCreateCustomerTokenRequest, InputItem,
    JSONSchemaFormat, MessageRole, MessageRoleExt, Model, OutputFormat, OutputFormatKind,
    OutputItem, Response, StopReason, StreamEvent, StreamEventKind, SubscriptionStatusKind, Tool,
    ToolCall, ToolCallDelta, ToolChoice, ToolChoiceType, ToolType, Usage, UsageSummary,
    WebToolConfig, WebToolIntent, XSearchConfig,
};

pub use bash_policy::BashPolicy;
pub use bash_tokenizer::{tokenize_bash, BashTokens};
pub use client::{
    AuthClient, Client, ClientBuilder, Config, CustomerResponsesClient, CustomerScopedClient,
    ResponsesClient,
};
pub use generated::{
    ImageData, ImagePinResponse, ImageRequest, ImageResponse, ImageResponseFormat, ImageUsage,
};
pub use generated::{RunsPendingToolCallV0, RunsPendingToolsResponse};
pub use generated::{
    SessionCreateRequest, SessionListResponse, SessionMessageCreateRequest, SessionMessageResponse,
    SessionResponse, SessionWithMessagesResponse,
};
pub use generated::{ToolCallId, ToolName};
pub use identifiers::TierCode;
pub use images::ImagesClient;
pub use local_bash_tools::{
    new_local_bash_tools, with_bash_allow_env_vars, with_bash_hard_max_output_bytes,
    with_bash_inherit_env, with_bash_max_output_bytes, with_bash_policy, with_bash_timeout,
    BashResult, LocalBashOption, LocalBashToolPack,
};
pub use local_fs_tools::{
    new_local_fs_tools, with_local_fs_hard_max_list_entries, with_local_fs_hard_max_read_bytes,
    with_local_fs_hard_max_search_matches, with_local_fs_ignore_dirs,
    with_local_fs_max_list_entries, with_local_fs_max_read_bytes, with_local_fs_max_search_bytes,
    with_local_fs_max_search_matches, with_local_fs_search_timeout, LocalFSOption, LocalFSToolPack,
};
pub use local_tools_common::LocalToolError;
#[cfg(feature = "streaming")]
pub use runs::RunEventStreamHandle;
pub use runs::{
    RunsClient, RunsCreateOptions, RunsCreateResponse, RunsGetResponse, RunsToolCallV0,
    RunsToolResultItemV0, RunsToolResultsRequest, RunsToolResultsResponse,
};
pub use sessions::{ListSessionsOptions, SessionsClient};
pub use tiers::{
    PriceInterval, Tier, TierCheckoutRequest, TierCheckoutSession, TierModel, TiersClient,
};
pub use workflow::{
    run_node_ref, ArtifactKey, EnvelopeVersion, ModelId, NodeErrorV0, NodeId, NodeResultV0,
    NodeStatusV0, NodeTypeV1, PayloadInfoV0, PlanHash, ProviderId, RequestId, RunCostLineItemV0,
    RunCostSummaryV0, RunEventEnvelope, RunEventPayload, RunEventTypeV0, RunEventV0, RunId,
    RunStatusV0, Sha256Hash, LLM_TEXT_OUTPUT, LLM_USER_MESSAGE_TEXT, RUN_EVENT_V0_SCHEMA_JSON,
};
pub use workflow_intent::{
    IntentKind, IntentSpec, WorkflowIntentCondition, WorkflowIntentKind, WorkflowIntentNode,
    WorkflowIntentNodeType, WorkflowIntentOutputRef, WorkflowIntentSpec,
    WorkflowIntentToolExecution, WorkflowIntentToolExecutionMode, WorkflowIntentToolRef,
    WorkflowIntentTransformValue,
};
pub use workflow_intent_builder::{
    chain, llm, parallel, workflow, workflow_intent, ChainOptions, JoinCollectOptions,
    LLMNodeBuilder, MapFanoutOptions, ParallelOptions, WorkflowIntentBuilder,
};
pub use workflows::{WorkflowsClient, WorkflowsCompileResponse, WorkflowsCompileResult};

// Workflow pattern helpers

// Structured output API
pub use structured::{
    output_format_from_type, AttemptRecord, DefaultRetryHandler, RetryHandler,
    StructuredDecodeError, StructuredError, StructuredErrorKind, StructuredExhaustedError,
    StructuredOptions, StructuredResult, ValidationIssue,
};

// Token providers for backendless auth
pub use token_providers::{
    poll_until, CustomerTokenProvider, CustomerTokenProviderConfig, CustomerTokenResponse,
    PollResult, PollUntilOptions, TokenProvider,
};

#[cfg(feature = "streaming")]
mod ndjson;
#[cfg(feature = "streaming")]
pub use ndjson::StreamHandle;

#[cfg(feature = "blocking")]
mod blocking;
#[cfg(all(feature = "blocking", feature = "billing"))]
pub use blocking::BlockingBillingClient;
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use blocking::BlockingRunEventStreamHandle;
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use blocking::BlockingStreamHandle;
#[cfg(feature = "blocking")]
pub use blocking::{
    BlockingAuthClient, BlockingClient, BlockingConfig, BlockingResponsesClient, BlockingRunsClient,
};
