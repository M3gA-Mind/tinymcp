//! TinyMCP-owned migrations composed with the pure bus collection declarations.

use crate::storage::local::{Owner, Schema, ScopeClass};
use tinymcp_bus::storage::{SCHEMA_VERSION, collection_specs};

/// Owner of all MCP documents and migration progress.
pub const OWNER: &str = "mcp";
/// Audit stream storing tool write records.
pub const AUDIT_STREAM: &str = "mcp.audit";
pub(crate) const SERVERS: &str = "mcp.servers";
pub(crate) const SERVER_NAMES: &str = "mcp.server_names";
pub(crate) const REGISTRY_CACHE: &str = "mcp.registry_cache";
pub(crate) const TOOL_CACHE: &str = "mcp.tool_cache";
pub(crate) const REGISTRY_INDEX: &str = "mcp.registry_index";
pub(crate) const REGISTRY_INDEX_STATE: &str = "mcp.registry_index_state";

/// MCP's owned collection and schema migrations.
pub(crate) fn storage_schema() -> tinystoragedrivers_types::Result<Schema> {
    let mut schema = Schema::new(Owner::new(OWNER)?);
    for spec in collection_specs() {
        schema = schema.collection_with(SCHEMA_VERSION, spec, ScopeClass::Tenant);
    }
    for step in super::migrate::steps() {
        schema = schema.step(step);
    }
    Ok(schema)
}

/// A document id made from escaped key components.
pub(crate) fn doc_id(parts: &[&str]) -> tinystoragedrivers_types::Result<String> {
    super::local::schema::composite_key(parts).map(|key| key.as_str().to_owned())
}

/// The schema version every MCP document write declares.
#[must_use]
pub(crate) fn write_options() -> tinystoragedrivers_types::WriteOptions {
    tinystoragedrivers_types::WriteOptions::new().schema(SCHEMA_VERSION)
}
