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
mod client;
mod core;
mod customers;
mod errors;
mod http;
mod identifiers;
mod local_fs_tools;
#[cfg(feature = "mock")]
mod mock;
mod models;
mod responses;
mod runs;
mod structured;
mod telemetry;
mod tiers;
mod token_providers;
pub mod tools;
mod types;
mod workflow;
mod workflow_builder;
mod workflows;

// Re-export common types used in public API for user convenience
pub use chrono::{DateTime, Utc};
pub use uuid::Uuid;

pub use api_key::{ApiKey, PublishableKey, SecretKey};
pub use errors::{
    APIError, Error, FieldError, RetryMetadata, TransportError, TransportErrorKind,
    ValidationError, WorkflowValidationError, WorkflowValidationIssue,
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
    DeviceFlowErrorKind, DeviceFlowProvider, DeviceStartRequest, DeviceTokenPending,
    DeviceTokenResponse, DeviceTokenResult, FunctionCall, FunctionCallDelta, FunctionTool,
    InputItem, JSONSchemaFormat, MessageRole, MessageRoleExt, Model, OutputFormat,
    OutputFormatKind, OutputItem, Response, StopReason, StreamEvent, StreamEventKind, Tool,
    ToolCall, ToolCallDelta, ToolChoice, ToolChoiceType, ToolType, Usage, UsageSummary,
    WebToolConfig, XSearchConfig,
};
// Re-export generated DeviceStartResponse for public API (interval polling config)
pub use generated::DeviceStartResponse;

pub use client::{
    AuthClient, Client, ClientBuilder, Config, CustomerResponsesClient, CustomerScopedClient,
    ResponsesClient,
};
pub use customers::{
    CheckoutSession, CheckoutSessionRequest, Customer, CustomerClaimRequest, CustomerCreateRequest,
    CustomerMetadata, CustomerUpsertRequest, CustomersClient, SubscriptionStatus,
    SubscriptionStatusKind,
};
pub use generated::{RunsPendingToolCallV0, RunsPendingToolsNodeV0, RunsPendingToolsResponse};
pub use identifiers::TierCode;
pub use local_fs_tools::{
    new_local_fs_tools, with_local_fs_hard_max_list_entries, with_local_fs_hard_max_read_bytes,
    with_local_fs_hard_max_search_matches, with_local_fs_ignore_dirs,
    with_local_fs_max_list_entries, with_local_fs_max_read_bytes, with_local_fs_max_search_bytes,
    with_local_fs_max_search_matches, with_local_fs_search_timeout, LocalFSOption, LocalFSToolPack,
};
pub use models::{CatalogModel, ModelsClient};
#[cfg(feature = "streaming")]
pub use runs::RunEventStreamHandle;
pub use runs::{RunsClient, RunsCreateResponse, RunsGetResponse};
pub use tiers::{
    PriceInterval, Tier, TierCheckoutRequest, TierCheckoutSession, TierModel, TiersClient,
};
pub use workflow::{
    run_node_ref, ArtifactKey, EdgeV0, EnvelopeVersion, ExecutionV0, ModelId, NodeErrorV0, NodeId,
    NodeResultV0, NodeStatusV0, NodeTypeV0, NodeV0, OutputRefV0, PayloadInfoV0, PlanHash,
    ProviderId, RequestId, RunCostLineItemV0, RunCostSummaryV0, RunEventEnvelope, RunEventPayload,
    RunEventTypeV0, RunEventV0, RunId, RunStatusV0, Sha256Hash, WorkflowKind, WorkflowSpecV0,
    RUN_EVENT_V0_SCHEMA_JSON, WORKFLOW_V0_SCHEMA_JSON,
};
pub use workflows::{WorkflowsClient, WorkflowsCompileResponseV0, WorkflowsCompileResultV0};

pub use workflow_builder::{
    workflow_v0, LlmResponsesBindingEncodingV0, LlmResponsesBindingV0, TransformJsonInputV0,
    TransformJsonValueV0, WorkflowBuilderV0,
};

// Structured output API
pub use structured::{
    output_format_from_type, AttemptRecord, DefaultRetryHandler, RetryHandler,
    StructuredDecodeError, StructuredError, StructuredErrorKind, StructuredExhaustedError,
    StructuredOptions, StructuredResult, ValidationIssue,
};

// Token providers for backendless auth
pub use token_providers::{
    poll_device_token, run_device_flow_for_id_token, start_device_authorization,
    CustomerTokenResponse, DeviceAuthConfig, DeviceAuthorization, DevicePollConfig, DeviceToken,
    IdTokenSource, OIDCExchangeConfig, OIDCExchangeTokenProvider, TokenProvider,
};

#[cfg(feature = "streaming")]
mod ndjson;
#[cfg(feature = "streaming")]
pub use ndjson::StreamHandle;

#[cfg(feature = "blocking")]
mod blocking;
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use blocking::BlockingRunEventStreamHandle;
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use blocking::BlockingStreamHandle;
#[cfg(feature = "blocking")]
pub use blocking::{
    BlockingAuthClient, BlockingClient, BlockingConfig, BlockingCustomersClient,
    BlockingResponsesClient, BlockingRunsClient, BlockingTiersClient,
};
