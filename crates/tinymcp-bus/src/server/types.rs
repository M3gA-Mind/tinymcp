//! Serialized vocabulary for module-owned server operations.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Host-owned declarations frozen for one server session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerSessionConfig {
    /// Caller-generated nonzero UUID-encoded integer counter, known before acquisition.
    /// Counters increase per registry; a 4096-counter window allows reordered opens.
    pub session_id: String,
    /// Server identity, version and optional instructions.
    pub info: Value,
    /// Host-selected provenance prefix.
    pub source_type_prefix: String,
    /// MCP resource declarations served by resources/list.
    pub resources: Vec<Value>,
}

/// A validated operation awaiting host policy and execution.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServerHostCall {
    /// List the tools the host permits this client to see.
    ListTools {
        /// Module-derived client provenance.
        source_type: String,
        /// Transport headers supplied with this operation.
        headers: Map<String, Value>,
    },
    /// Apply approvals and dispatch one tool.
    CallTool {
        /// Module-derived client provenance.
        source_type: String,
        /// Transport headers supplied with this operation.
        headers: Map<String, Value>,
        /// Validated tool name.
        name: String,
        /// Normalized argument object.
        arguments: Map<String, Value>,
    },
    /// Read a declared resource through the host's domain dispatcher.
    ReadResource {
        /// Module-derived client provenance.
        source_type: String,
        /// Transport headers supplied with this operation.
        headers: Map<String, Value>,
        /// Resource URI validated by the protocol.
        uri: String,
    },
    /// List host-owned prompt declarations.
    ListPrompts {
        /// Module-derived client provenance.
        source_type: String,
        /// Transport headers supplied with this operation.
        headers: Map<String, Value>,
    },
    /// Resolve a named host-owned prompt.
    GetPrompt {
        /// Module-derived client provenance.
        source_type: String,
        /// Transport headers supplied with this operation.
        headers: Map<String, Value>,
        /// Validated prompt name.
        name: String,
        /// String-valued MCP prompt arguments.
        arguments: Map<String, Value>,
    },
    /// A newer host requested an operation this module does not support.
    #[serde(other)]
    Unsupported,
}

/// A host's result after applying its own policy and domain operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerHostReply {
    /// A successful MCP result, or an array of MCP tool declarations for list.
    Success {
        /// Result payload.
        value: Value,
    },
    /// A caller-correctable error.
    InvalidParams {
        /// Human-readable detail.
        message: String,
    },
    /// A host-side failure.
    Internal {
        /// Human-readable detail.
        message: String,
    },
    /// A URI unavailable in the host's resource domain.
    ResourceNotFound {
        /// Human-readable detail.
        message: String,
    },
}

/// One callback emitted by a pending server operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerCallback {
    /// Opaque, single-use callback identifier.
    pub id: String,
    /// Validated domain operation.
    pub call: ServerHostCall,
}

/// The current observable state of a submitted input line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServerOperationState {
    /// Processing input, or waiting for a previously drained callback reply.
    Pending,
    /// Apply host policy and reply through `ServerComplete`.
    Callback {
        /// Single outstanding host callback.
        callback: ServerCallback,
    },
    /// Finished; remains replayable until the next operation acknowledges it.
    Complete {
        /// Framed JSON-RPC response, absent for notifications.
        response: Option<String>,
    },
    /// Cancelled; remains replayable until the next operation acknowledges it.
    Cancelled,
    /// A module operation could not produce a bounded protocol response.
    Failed {
        /// Stable module-owned failure description.
        message: String,
    },
    /// A newer module reported an operation state this host does not know.
    #[serde(other)]
    Unknown,
}

impl std::fmt::Debug for ServerHostCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::ListTools { .. } => "ListTools",
            Self::CallTool { .. } => "CallTool",
            Self::ReadResource { .. } => "ReadResource",
            Self::ListPrompts { .. } => "ListPrompts",
            Self::GetPrompt { .. } => "GetPrompt",
            Self::Unsupported => "Unsupported",
        };
        f.debug_struct(kind).finish_non_exhaustive()
    }
}

/// Input identity known before submission, allowing side-effect-safe retries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerInput {
    /// Caller-generated nonzero UUID-encoded integer counter, increasing strictly per session.
    pub operation_id: String,
    /// One newline-delimited MCP message or batch.
    pub line: String,
}

/// Replayable observation of one caller-known operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerOperationSnapshot {
    /// Identifies the operation even when a poll reply arrives late.
    pub operation_id: String,
    /// Current callback or terminal state.
    pub state: ServerOperationState,
    /// HTTP response metadata for this exact operation, when it used HTTP.
    pub http_response: Option<ServerHttpResponseMetadata>,
}

/// Module-owned HTTP session state, queried without inspecting JSON-RPC output.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerSessionMetadata {
    /// Whether the module successfully handled an `initialize` request.
    pub initialized: bool,
    /// The version selected by the module, present after initialization.
    pub protocol_version: Option<String>,
}

impl std::fmt::Debug for ServerSessionMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerSessionMetadata")
            .field("initialized", &self.initialized)
            .field("protocol_version", &self.protocol_version)
            .finish_non_exhaustive()
    }
}

/// HTTP status and transport headers selected by the module for one operation.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerHttpResponseMetadata {
    /// HTTP response status, as a standard numeric status code.
    pub status_code: u16,
    /// Transport headers, including negotiated session headers when applicable.
    pub headers: BTreeMap<String, String>,
    /// Content type for a transport-level rejection body.
    pub content_type: Option<String>,
    /// Plain-text body for a transport-level rejection, if any.
    pub error_body: Option<String>,
}

impl std::fmt::Debug for ServerHttpResponseMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerHttpResponseMetadata")
            .field("status_code", &self.status_code)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("content_type", &self.content_type)
            .field("has_error_body", &self.error_body.is_some())
            .finish_non_exhaustive()
    }
}

/// HTTP admission context for one module-owned server session.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerHttpSessionConfig {
    /// The server identity, provenance and resource declarations.
    pub server: ServerSessionConfig,
    /// The session ID supplied by the client, or `None` for an initial request.
    pub requested_session_id: Option<String>,
    /// Whether the host's HTTP session table recognizes the supplied ID.
    pub known_session: bool,
    /// The protocol version the known session negotiated, if one exists.
    pub expected_protocol_version: Option<String>,
}

/// HTTP session operation whose admission is decided by the module.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerHttpSessionMethod {
    /// Opens the event stream for an existing session.
    Get,
    /// Closes an existing session, idempotently.
    Delete,
}

/// Headers and host session-table lookup supplied to a module-owned HTTP decision.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerHttpSessionRequest {
    /// HTTP session operation.
    pub method: ServerHttpSessionMethod,
    /// Session ID supplied by the remote client, when present.
    pub requested_session_id: Option<String>,
    /// Internal server-session ID if the host session table recognizes the wire ID.
    pub internal_session_id: Option<String>,
    /// Raw request headers; the module owns header matching and protocol checks.
    pub headers: Map<String, Value>,
}

impl std::fmt::Debug for ServerHttpSessionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerHttpSessionRequest")
            .field("method", &self.method)
            .field(
                "has_requested_session_id",
                &self.requested_session_id.is_some(),
            )
            .field(
                "has_internal_session_id",
                &self.internal_session_id.is_some(),
            )
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ServerHttpSessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerHttpSessionConfig")
            .field("server", &self.server)
            .field(
                "has_requested_session_id",
                &self.requested_session_id.is_some(),
            )
            .field("known_session", &self.known_session)
            .field(
                "has_expected_protocol_version",
                &self.expected_protocol_version.is_some(),
            )
            .finish_non_exhaustive()
    }
}

/// Cancellation targets the operation known before submission, never its successor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerOperationRef {
    /// Caller-known session UUID.
    pub session_id: String,
    /// Caller-known operation UUID.
    pub operation_id: String,
}
