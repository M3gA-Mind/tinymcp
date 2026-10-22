//! Pure conversions for records written by the previous storage schema.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use super::schema::{AUDIT_STREAM, doc_id};

/// A server document written by the previous `mcp_servers` collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct LegacyServerDocument {
    /// The document key in the old collection.
    pub(super) storage_id: String,
    /// The old server body, including any fields unknown to this contract.
    pub(super) body: LegacyServerBody,
}

/// The previous release's server record shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct LegacyServerBody {
    /// The server's stable owner ID.
    pub server_id: String,
    /// Its qualified registry name.
    pub qualified_name: String,
    /// Its display name.
    pub display_name: String,
    /// Optional user-facing description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional registry icon URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// The command launcher kind.
    pub command_kind: String,
    /// The command or executable.
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Names of environment variables required by the server.
    #[serde(default)]
    pub env_keys: Vec<String>,
    /// Optional opaque configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    /// Installation timestamp in the old field.
    pub installed_at: i64,
    /// Optional last successful connection timestamp in the old field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_at: Option<i64>,
    /// Optional transport discriminator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// Optional deployment URL for remote transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_url: Option<String>,
    /// Whether the server is enabled when the field was present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Whether the server came from a managed catalog when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_managed: Option<bool>,
    /// Unrecognized fields retained verbatim by the conversion.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A server record in the current `mcp.servers` document shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct ServerDocument {
    /// The composite-key-escaped document ID.
    pub(super) id: String,
    /// The current server body.
    pub(super) body: ServerDocumentBody,
}

/// The current owner document body for an installed MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct ServerDocumentBody {
    /// The server's stable owner ID.
    pub server_id: String,
    /// Its qualified registry name.
    pub qualified_name: String,
    /// Its display name.
    pub display_name: String,
    /// Optional user-facing description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional registry icon URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// The command launcher kind.
    pub command_kind: String,
    /// The command or executable.
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Names of environment variables required by the server.
    #[serde(default)]
    pub env_keys: Vec<String>,
    /// Optional opaque configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    /// Installation timestamp in Unix milliseconds.
    pub installed_ms: i64,
    /// Optional last successful connection timestamp in Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_ms: Option<i64>,
    /// Optional transport discriminator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// Optional deployment URL for remote transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_url: Option<String>,
    /// Whether the server is enabled when the field was present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Whether the server came from a managed catalog when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_managed: Option<bool>,
    /// Unrecognized fields retained verbatim by the conversion.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Converts an old server record to its current collection ID and body.
///
/// # Errors
///
/// Returns a storage error when the old body is malformed or its ID cannot be
/// represented as a storage key.
pub(super) fn convert_legacy_server(
    record: LegacyServerDocument,
) -> tinystoragedrivers_types::Result<ServerDocument> {
    let body = record.body;
    let id = doc_id(&[&body.server_id])?;
    Ok(ServerDocument {
        id,
        body: ServerDocumentBody {
            server_id: body.server_id,
            qualified_name: body.qualified_name,
            display_name: body.display_name,
            description: body.description,
            icon_url: body.icon_url,
            command_kind: body.command_kind,
            command: body.command,
            args: body.args,
            env_keys: body.env_keys,
            config: body.config,
            installed_ms: body.installed_at,
            last_connected_ms: body.last_connected_at,
            transport: body.transport,
            deployment_url: body.deployment_url,
            enabled: body.enabled,
            catalog_managed: body.catalog_managed,
            extra: body.extra,
        },
    })
}

/// A qualified-name claim written by the previous collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct LegacyServerNameClaim {
    /// The qualified name held by this claim.
    pub qualified_name: String,
    /// The server ID that owns the claim.
    pub server_id: String,
    /// Optional timestamp in the old field.
    #[serde(default)]
    pub claimed_at: Option<i64>,
    /// Unknown fields retained by conversion.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A qualified-name claim in the current `mcp.server_names` collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ServerNameClaim {
    /// The composite-key-escaped ID, derived from `qualified_name`.
    pub(super) id: String,
    /// The current owner document body.
    pub(super) body: ServerNameClaimBody,
}

/// The current owner body for one qualified-name claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ServerNameClaimBody {
    /// The qualified name held by this claim.
    pub qualified_name: String,
    /// The server ID that owns the claim.
    pub server_id: String,
    /// A missing legacy timestamp means the claim was already expired.
    pub claimed_ms: i64,
    /// Unknown fields retained by conversion.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Converts an old qualified-name claim into the current document shape.
///
/// # Errors
///
/// Returns a storage error when the qualified name cannot be represented as
/// a storage key.
pub(super) fn convert_legacy_server_name_claim(
    claim: LegacyServerNameClaim,
) -> tinystoragedrivers_types::Result<ServerNameClaim> {
    Ok(ServerNameClaim {
        id: doc_id(&[&claim.qualified_name])?,
        body: ServerNameClaimBody {
            qualified_name: claim.qualified_name,
            server_id: claim.server_id,
            claimed_ms: claim.claimed_at.unwrap_or(0),
            extra: claim.extra,
        },
    })
}

/// Maps an old audit stream family to the current owner stream family.
///
/// Only the exact legacy family and its slash-delimited descendants match.
#[must_use]
pub(super) fn legacy_audit_stream_target(source: &str) -> Option<String> {
    if source == "mcp_audit" {
        Some(AUDIT_STREAM.to_owned())
    } else {
        source
            .strip_prefix("mcp_audit/")
            .map(|suffix| format!("{AUDIT_STREAM}/{suffix}"))
    }
}
