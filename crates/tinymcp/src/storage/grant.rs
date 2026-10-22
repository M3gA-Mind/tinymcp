//! What the module declares it needs from storage.
//!
//! The collections and the stream come from the private `storage_schema` helper, so the grant
//! cannot drift from what the module declares. Only the secret names are
//! listed here: a secret is not renamed by a migration step, and a name outside
//! the `mcp/` owner prefix cannot be declared as a schema item.

use serde::Serialize;
use tinystoragedrivers_types::CollectionSpec;

use super::schema::AUDIT_STREAM;

/// Prefix of the secret holding one server's environment and OAuth values:
/// the full name is this prefix plus the install identifier.
pub const ENV_SECRET_PREFIX: &str = "mcp_client_env/";
/// The secret holding the key the tool-cache fingerprints are keyed with.
pub const CACHE_KEY_SECRET: &str = "mcp_cache_fingerprint_key";

const LEGACY_COLLECTIONS: [&str; 6] = [
    "mcp_servers",
    "mcp_server_names",
    "mcp_registry_cache",
    "mcp_tool_cache",
    "mcp_registry_index",
    "mcp_registry_index_state",
];
const LEGACY_AUDIT_STREAM: &str = "mcp_audit";

/// Everything the module reads or writes through the storage ports.
///
/// A host that serves storage to this module over the bus uses it as the
/// module's allowlist: the collections (with the indexes they need), the
/// stream, and the secret names. Anything outside it is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageGrant {
    /// Document collections the module uses.
    pub collections: Vec<String>,
    /// Collections that need full-text search.
    pub searchable: Vec<String>,
    /// Stream names (exact) the module appends to and reads.
    pub streams: Vec<String>,
    /// Secret names the module uses, as exact names or `prefix*` patterns.
    pub secrets: Vec<String>,
}

/// The module's storage grant, generated from the private `storage_schema` helper.
#[must_use]
pub fn grant() -> StorageGrant {
    let mut collections: Vec<String> = collection_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    collections.extend(LEGACY_COLLECTIONS.map(str::to_owned));
    StorageGrant {
        collections,
        searchable: Vec::new(),
        streams: vec![AUDIT_STREAM.to_owned(), LEGACY_AUDIT_STREAM.to_owned()],
        secrets: vec![
            format!("{ENV_SECRET_PREFIX}*"),
            CACHE_KEY_SECRET.to_string(),
        ],
    }
}

/// The collections the module declares, with their indexes.
#[must_use]
pub fn collection_specs() -> Vec<CollectionSpec> {
    tinymcp_bus::storage::collection_specs()
}
