//! Pure collection names and shapes for MCP storage.

use tinystoragedrivers_types::{CollectionSpec, IndexSpec, WriteOptions};

/// The owner every name below starts with.
pub const OWNER: &str = "mcp";

/// The body version every collection is declared at.
pub const SCHEMA_VERSION: u32 = 2;

/// One document per installed server.
pub const SERVERS: &str = "mcp.servers";
/// Claims on qualified names, so installing one service twice is one install.
pub const SERVER_NAMES: &str = "mcp.server_names";
/// Upstream browse responses, with a timestamp.
pub const REGISTRY_CACHE: &str = "mcp.registry_cache";
/// Each server's last advertised tool list, keyed by a definition fingerprint.
pub const TOOL_CACHE: &str = "mcp.tool_cache";
/// The local copy of an upstream catalog, one document per server.
pub const REGISTRY_INDEX: &str = "mcp.registry_index";
/// Where each catalog's sync stands.
pub const REGISTRY_INDEX_STATE: &str = "mcp.registry_index_state";
/// The append-only stream of tool-write audit records.
pub const AUDIT_STREAM: &str = "mcp.audit";
/// Shared progress records used by MCP's owner-local migrations.
pub const MIGRATIONS: &str = "tsd.migrations";

/// Collection declarations the module asks the host to provision.
#[must_use]
pub fn collection_specs() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(SERVERS)
            .index(IndexSpec::new("qualified_name", ["qualified_name"]))
            .index(IndexSpec::new("installed_ms", ["installed_ms"])),
        CollectionSpec::new(SERVER_NAMES),
        CollectionSpec::new(REGISTRY_CACHE).index(IndexSpec::new("cache_key", ["cache_key"])),
        CollectionSpec::new(TOOL_CACHE),
        CollectionSpec::new(REGISTRY_INDEX).index(IndexSpec::new("source", ["source"])),
        CollectionSpec::new(REGISTRY_INDEX_STATE),
        CollectionSpec::new(MIGRATIONS),
    ]
}

/// The options every document write carries: the body version.
#[must_use]
pub fn write_options() -> WriteOptions {
    WriteOptions::new().schema(SCHEMA_VERSION)
}

