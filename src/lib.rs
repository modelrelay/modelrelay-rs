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
#[cfg(feature = "billing")]
mod billing;
mod client;
mod core;
mod errors;
mod http;
mod identifiers;
mod images;
mod local_fs_tools;
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
mod workflow_builder;
mod workflow_patterns;
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
    FunctionCall, FunctionCallDelta, FunctionTool, InputItem, JSONSchemaFormat, MessageRole,
    MessageRoleExt, Model, OutputFormat, OutputFormatKind, OutputItem, Response, StopReason,
    StreamEvent, StreamEventKind, SubscriptionStatusKind, Tool, ToolCall, ToolCallDelta,
    ToolChoice, ToolChoiceType, ToolType, Usage, UsageSummary, WebToolConfig, WebToolIntent,
    XSearchConfig,
};

pub use client::{
    AuthClient, Client, ClientBuilder, Config, CustomerResponsesClient, CustomerScopedClient,
    ResponsesClient,
};
pub use generated::{
    ImageData, ImagePinResponse, ImageRequest, ImageResponse, ImageResponseFormat, ImageUsage,
};
pub use generated::{RunsPendingToolCallV0, RunsPendingToolsNodeV0, RunsPendingToolsResponse};
pub use generated::{
    SessionCreateRequest, SessionListResponse, SessionMessageCreateRequest, SessionMessageResponse,
    SessionResponse, SessionWithMessagesResponse,
};
pub use identifiers::TierCode;
pub use images::ImagesClient;
pub use local_fs_tools::{
    new_local_fs_tools, with_local_fs_hard_max_list_entries, with_local_fs_hard_max_read_bytes,
    with_local_fs_hard_max_search_matches, with_local_fs_ignore_dirs,
    with_local_fs_max_list_entries, with_local_fs_max_read_bytes, with_local_fs_max_search_bytes,
    with_local_fs_max_search_matches, with_local_fs_search_timeout, LocalFSOption, LocalFSToolPack,
};
#[cfg(feature = "streaming")]
pub use runs::RunEventStreamHandle;
pub use runs::{RunsClient, RunsCreateResponse, RunsGetResponse};
pub use sessions::{ListSessionsOptions, SessionsClient};
pub use tiers::{
    PriceInterval, Tier, TierCheckoutRequest, TierCheckoutSession, TierModel, TiersClient,
};
pub use workflow::{
    run_node_ref, ArtifactKey, ConditionOpV1, ConditionSourceV1, ConditionV1, EdgeV0, EdgeV1,
    EnvelopeVersion, ExecutionV0, ExecutionV1, ModelId, NodeErrorV0, NodeId, NodeResultV0,
    NodeStatusV0, NodeTypeV0, NodeTypeV1, NodeV0, NodeV1, OutputRefV0, OutputRefV1, PayloadInfoV0,
    PlanHash, ProviderId, RequestId, RunCostLineItemV0, RunCostSummaryV0, RunEventEnvelope,
    RunEventPayload, RunEventTypeV0, RunEventV0, RunId, RunStatusV0, Sha256Hash, WorkflowKind,
    WorkflowSpecV0, WorkflowSpecV1, RUN_EVENT_V0_SCHEMA_JSON, WORKFLOW_V0_SCHEMA_JSON,
    WORKFLOW_V1_SCHEMA_JSON,
};
// Workflow.v1 condition and binding helper functions
pub use workflow::{
    bind_to_placeholder, bind_to_placeholder_with_pointer, bind_to_pointer,
    bind_to_pointer_with_source, when_output_equals, when_output_exists, when_output_matches,
    when_status_equals, when_status_exists, when_status_matches, BindingBuilder,
};
pub use workflows::{
    WorkflowsClient, WorkflowsCompileResponseV0, WorkflowsCompileResponseV1,
    WorkflowsCompileResultV0, WorkflowsCompileResultV1,
};

pub use workflow_builder::{
    join_output_text, new_workflow, workflow_v0, workflow_v1, JoinAnyInputV1, JoinCollectInputV1,
    LlmNodeBuilder, LlmResponsesBindingEncodingV0, LlmResponsesBindingEncodingV1,
    LlmResponsesBindingV0, LlmResponsesBindingV1, LlmResponsesNodeOptionsV1,
    LlmResponsesToolLimitsV0, LlmResponsesToolLimitsV1, MapFanoutInputV1, MapFanoutItemBindingV1,
    MapFanoutItemsV1, MapFanoutSubNodeV1, ToolExecutionModeV0, ToolExecutionModeV1,
    TransformJsonInputV0, TransformJsonInputV1, TransformJsonNodeBuilder, TransformJsonValueV0,
    TransformJsonValueV1, Workflow, WorkflowBuilderV0, WorkflowBuilderV1, LLM_TEXT_OUTPUT,
    LLM_USER_MESSAGE_TEXT,
};

// Workflow pattern helpers
pub use workflow_patterns::{Chain, LLMStep, MapItem, MapReduce, Parallel};

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
