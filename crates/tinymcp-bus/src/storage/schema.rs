//! Pure collection names and shapes for MCP storage.

use sha2::{Digest, Sha256};
use tinystoragedrivers_types::{CollectionSpec, IndexSpec, Key, MAX_ID_LEN, WriteOptions};

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

/// The document id for `parts`, using the released TSD composite-key encoding.
///
/// # Errors
///
/// Returns the storage error for an empty key.
pub fn doc_id(parts: &[&str]) -> tinystoragedrivers_types::Result<String> {
    if parts.is_empty() {
        return Err(tinystoragedrivers_types::StorageError::invalid_input(
            "a key needs at least one part",
        ));
    }
    let mut encoded = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index != 0 {
            encoded.push('/');
        }
        for (character_index, character) in part.chars().enumerate() {
            match character {
                '%' => encoded.push_str("%25"),
                '/' => encoded.push_str("%2F"),
                '\0' => encoded.push_str("%00"),
                '~' if index == 0 && character_index == 0 => encoded.push_str("%7E"),
                other => encoded.push(other),
            }
        }
    }
    if encoded.len() > MAX_ID_LEN {
        encoded = format!(
            "~{}",
            data_encoding::BASE32_NOPAD
                .encode(&Sha256::digest(encoded.as_bytes()))
                .to_ascii_lowercase()
        );
    }
    Key::new(encoded).map(|key| key.as_str().to_owned())
}
