//! Pure conversions for records written by the previous storage schema.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use super::schema::{AUDIT_STREAM, doc_id};

/// A server document written by the previous `mcp_servers` collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyServerDocument {
    /// The document key in the old collection.
    pub storage_id: String,
    /// The old server body, including any fields unknown to this contract.
    pub body: LegacyServerBody,
}

/// The previous release's server record shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyServerBody {
    pub server_id: String,
    pub qualified_name: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub command_kind: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    pub installed_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_managed: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A server record in the current `mcp.servers` document shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerDocument {
    /// The composite-key-escaped document ID.
    pub id: String,
    /// The current server body.
    pub body: ServerDocumentBody,
}

/// The current owner document body for an installed MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerDocumentBody {
    pub server_id: String,
    pub qualified_name: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub command_kind: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    pub installed_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_managed: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Converts an old server record to its current collection ID and body.
///
/// # Errors
///
/// Returns a storage error when the old body is malformed or its ID cannot be
/// represented as a storage key.
pub fn convert_legacy_server(
    record: LegacyServerDocument,
) -> tinystoragedrivers_types::Result<ServerDocument> {
    let id = doc_id(&[&record.storage_id])?;
    let body = record.body;
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
pub struct LegacyServerNameClaim {
    pub qualified_name: String,
    pub server_id: String,
    #[serde(default)]
    pub claimed_at: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A qualified-name claim in the current `mcp.server_names` collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerNameClaim {
    /// The composite-key-escaped ID, derived from `qualified_name`.
    pub id: String,
    pub qualified_name: String,
    pub server_id: String,
    /// A missing legacy timestamp means the claim was already expired.
    pub claimed_ms: i64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Converts an old qualified-name claim into the current document shape.
///
/// # Errors
///
/// Returns a storage error when the qualified name cannot be represented as
/// a storage key.
pub fn convert_legacy_server_name_claim(
    claim: LegacyServerNameClaim,
) -> tinystoragedrivers_types::Result<ServerNameClaim> {
    Ok(ServerNameClaim {
        id: doc_id(&[&claim.qualified_name])?,
        qualified_name: claim.qualified_name,
        server_id: claim.server_id,
        claimed_ms: claim.claimed_at.unwrap_or(0),
        extra: claim.extra,
    })
}

/// Maps an old audit stream family to the current owner stream family.
///
/// Only the exact legacy family and its slash-delimited descendants match.
#[must_use]
pub fn legacy_audit_stream_target(source: &str) -> Option<String> {
    if source == "mcp_audit" {
        Some(AUDIT_STREAM.to_owned())
    } else {
        source
            .strip_prefix("mcp_audit/")
            .map(|suffix| format!("{AUDIT_STREAM}/{suffix}"))
    }
}
