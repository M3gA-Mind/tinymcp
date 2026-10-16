//! The module's storage declaration: what it keeps, under which names.
//!
//! [`storage_schema`] is the one place the module's collections and stream are
//! named. A host composes it with every other owner's schema
//! (`Registry::compose`), prepares the backend from it, runs its migration
//! steps over the module's scope, and generates the module's storage grant from
//! it (see [`grant`](super::grant)).
//!
//! # Names
//!
//! Every name starts with the owner `mcp`. Documents use bare `snake_case`
//! bodies, times are epoch milliseconds in `*_ms` fields, and ids are built
//! with `Key::of`, so a qualified name such as `@acme/files` is stored as
//! `@acme%2Ffiles` and never needs a digest to be a safe id.
//!
//! | v1 | v2 |
//! | --- | --- |
//! | `mcp_servers` | `mcp.servers` |
//! | `mcp_server_names` | `mcp.server_names` |
//! | `mcp_registry_cache` | `mcp.registry_cache` |
//! | `mcp_tool_cache` | `mcp.tool_cache` |
//! | `mcp_registry_index` | `mcp.registry_index` |
//! | `mcp_registry_index_state` | `mcp.registry_index_state` |
//! | stream `mcp_audit` | stream `mcp.audit` |
//!
//! The secrets (`mcp_client_env/<server id>`, `mcp_cache_fingerprint_key`) keep
//! their names: a secret cannot be renamed by a migration step, and a name
//! outside the owner prefix cannot be declared as an item, so the grant lists
//! them beside the schema.

use tinystoragedrivers_core::schema::{Owner, Schema, ScopeClass};
use tinystoragedrivers_core::{CollectionSpec, IndexSpec, Key, WriteOptions};

use super::migrate;

/// The owner every name below starts with.
pub const OWNER: &str = "mcp";

/// The body version every collection is declared at. Version 1 is the layout
/// of the previous release, whose collections carried `mcp_` names.
pub(crate) const SCHEMA_VERSION: u32 = 2;

/// One document per installed server.
pub(crate) const SERVERS: &str = "mcp.servers";
/// Claims on qualified names, so installing one service twice is one install.
pub(crate) const SERVER_NAMES: &str = "mcp.server_names";
/// Upstream browse responses, with a timestamp.
pub(crate) const REGISTRY_CACHE: &str = "mcp.registry_cache";
/// Each server's last advertised tool list, keyed by a definition fingerprint.
pub(crate) const TOOL_CACHE: &str = "mcp.tool_cache";
/// The local copy of an upstream catalog, one document per server.
pub(crate) const REGISTRY_INDEX: &str = "mcp.registry_index";
/// Where each catalog's sync stands.
pub(crate) const REGISTRY_INDEX_STATE: &str = "mcp.registry_index_state";

/// The append-only stream of tool-write audit records.
pub const AUDIT_STREAM: &str = "mcp.audit";

/// Everything the module stores, with the steps that bring a v1 database
/// forward.
///
/// All items live in [`ScopeClass::Module`]: the module never names a tenant,
/// the handles it is given carry the scope.
///
/// # Panics
///
/// Never in practice: the owner name is a constant that `Owner::new` accepts,
/// and a unit test pins that.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "the owner is the constant `mcp`, which `Owner::new` accepts; a test pins it"
)]
pub fn storage_schema() -> Schema {
    let owner = Owner::new(OWNER).expect("`mcp` is a valid owner name");
    let schema = Schema::new(owner)
        .collection_with(
            SCHEMA_VERSION,
            CollectionSpec::new(SERVERS)
                .index(IndexSpec::new("qualified_name", ["qualified_name"]))
                .index(IndexSpec::new("installed_ms", ["installed_ms"])),
            ScopeClass::Module,
        )
        .collection(SERVER_NAMES, SCHEMA_VERSION, ScopeClass::Module)
        .collection_with(
            SCHEMA_VERSION,
            CollectionSpec::new(REGISTRY_CACHE).index(IndexSpec::new("cache_key", ["cache_key"])),
            ScopeClass::Module,
        )
        .collection(TOOL_CACHE, SCHEMA_VERSION, ScopeClass::Module)
        .collection_with(
            SCHEMA_VERSION,
            CollectionSpec::new(REGISTRY_INDEX).index(IndexSpec::new("source", ["source"])),
            ScopeClass::Module,
        )
        .collection(REGISTRY_INDEX_STATE, SCHEMA_VERSION, ScopeClass::Module)
        .stream(AUDIT_STREAM, ScopeClass::Module);
    migrate::steps().into_iter().fold(schema, Schema::step)
}

/// The options every document write carries: the body version.
pub(crate) fn write_options() -> WriteOptions {
    WriteOptions::new().schema(SCHEMA_VERSION)
}

/// The document id for `parts`: joined and escaped by [`Key::of`], and hashed
/// when it would be too long.
///
/// # Errors
///
/// Returns the storage error for an empty key.
pub(crate) fn doc_id(parts: &[&str]) -> tinystoragedrivers_core::Result<String> {
    Key::of(parts).map(|key| key.as_str().to_owned())
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
