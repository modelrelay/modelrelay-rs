//! Minimal Rust SDK for the ModelRelay API.
#![cfg_attr(docsrs, feature(doc_cfg))]
// Allow large error types - refactoring to Box<Error> would be a breaking change
#![allow(clippy::result_large_err)]

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
pub const REQUEST_ID_HEADER: &str = "X-ModelRelay-Chat-Request-Id";

/// HTTP header name for API key authentication.
pub(crate) const API_KEY_HEADER: &str = "X-ModelRelay-Api-Key";

#[cfg(any(feature = "client", feature = "blocking"))]
mod chat;
#[cfg(any(feature = "client", feature = "blocking"))]
mod core;
#[cfg(feature = "client")]
mod customers;
mod errors;
#[cfg(any(feature = "client", feature = "blocking"))]
mod http;
#[cfg(feature = "mock")]
mod mock;
#[cfg(feature = "client")]
mod structured;
mod telemetry;
#[cfg(feature = "client")]
mod tiers;
pub mod tools;
mod types;

#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use chat::BlockingStructuredJSONStream;
#[cfg(all(feature = "streaming", any(feature = "client", feature = "blocking")))]
pub use chat::ChatStreamAdapter;
#[cfg(all(feature = "client", feature = "streaming"))]
pub use chat::StructuredJSONStream;
#[cfg(any(feature = "client", feature = "blocking"))]
pub use chat::{
    ChatRequestBuilder, CustomerChatRequestBuilder, CustomerProxyRequestBody, CUSTOMER_ID_HEADER,
};
#[cfg(feature = "streaming")]
pub use chat::{StructuredJSONEvent, StructuredRecordKind};
#[cfg(any(feature = "client", feature = "blocking", feature = "streaming"))]
pub use errors::{
    APIError, Error, FieldError, RetryMetadata, TransportError, TransportErrorKind, ValidationError,
};
#[cfg(not(any(feature = "client", feature = "blocking", feature = "streaming")))]
pub use errors::{APIError, Error, FieldError, RetryMetadata, ValidationError};
#[cfg(any(feature = "client", feature = "blocking"))]
pub use http::{HeaderEntry, HeaderList, ProxyOptions, RetryConfig};
#[cfg(feature = "mock")]
pub use mock::{fixtures, MockAuthClient, MockClient, MockConfig, MockLLMClient};
pub use telemetry::{
    HttpRequestMetrics, MetricsCallbacks, RequestContext, StreamFirstTokenMetrics,
    TokenUsageMetrics,
};
pub use tools::{
    assistant_message_with_tool_calls, create_retry_messages, execute_with_retry,
    format_tool_error_for_model, get_retryable_errors, has_retryable_errors,
    parse_and_validate_tool_args, parse_tool_args, respond_to_tool_call, respond_to_tool_call_json,
    sync_handler, tool_result_message, tool_result_message_json, BoxFuture, ParseResult,
    ProxyResponseExt, RetryOptions, ToolArgsError, ToolCallAccumulator, ToolExecutionResult,
    ToolHandler, ToolRegistry, UnknownToolError, ValidateArgs,
};
pub use tools::{function_tool_from_type, ToolSchema};
pub use types::{
    APIKey, CodeExecConfig, FrontendToken, FrontendTokenAutoProvisionRequest, FrontendTokenRequest,
    FunctionCall, FunctionCallDelta, FunctionTool, MessageRole, Model, ProxyMessage, ProxyRequest,
    ProxyResponse, ResponseFormat, ResponseFormatKind, ResponseJSONSchema, StopReason, StreamEvent,
    StreamEventKind, TokenType, Tool, ToolCall, ToolCallDelta, ToolChoice, ToolChoiceType,
    ToolType, Usage, UsageSummary, WebToolConfig, XSearchConfig,
};

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
pub use client::{AuthClient, Client, ClientBuilder, Config, LLMClient};
#[cfg(feature = "client")]
pub use customers::{
    CheckoutSession, CheckoutSessionRequest, Customer, CustomerClaimRequest, CustomerCreateRequest,
    CustomerMetadata, CustomerUpsertRequest, CustomersClient, SubscriptionStatus,
};
#[cfg(feature = "client")]
pub use tiers::{PriceInterval, Tier, TierCheckoutRequest, TierCheckoutSession, TiersClient};

// Structured output API
#[cfg(feature = "client")]
pub use structured::{
    response_format_from_type, AttemptRecord, CustomerStructuredChatBuilder, DefaultRetryHandler,
    RetryHandler, StructuredChatBuilder, StructuredDecodeError, StructuredError,
    StructuredErrorKind, StructuredExhaustedError, StructuredOptions, StructuredResult,
    ValidationIssue,
};

#[cfg(all(feature = "client", feature = "streaming"))]
mod sse;
#[cfg(all(feature = "client", feature = "streaming"))]
pub use sse::StreamHandle;

#[cfg(feature = "blocking")]
mod blocking;
#[cfg(all(feature = "blocking", feature = "streaming"))]
pub use blocking::BlockingProxyHandle;
#[cfg(feature = "blocking")]
pub use blocking::{BlockingAuthClient, BlockingClient, BlockingConfig, BlockingLLMClient};
