//! What the module declares it needs from storage.

use serde::Serialize;
use tinystoragedrivers_core::{CollectionSpec, IndexSpec};

/// One document per installed server.
pub(crate) const SERVERS: &str = "mcp_servers";
/// Claims on qualified names, so installing one service twice is one install.
pub(crate) const SERVER_NAMES: &str = "mcp_server_names";
/// Upstream browse responses, with a timestamp.
pub(crate) const REGISTRY_CACHE: &str = "mcp_registry_cache";
/// Each server's last advertised tool list, keyed by a definition fingerprint.
pub(crate) const TOOL_CACHE: &str = "mcp_tool_cache";
/// The local copy of an upstream catalog, one document per server.
pub(crate) const REGISTRY_INDEX: &str = "mcp_registry_index";
/// Where each catalog's sync stands.
pub(crate) const REGISTRY_INDEX_STATE: &str = "mcp_registry_index_state";

/// The append-only stream of tool-write audit records.
pub const AUDIT_STREAM: &str = "mcp_audit";
/// Prefix of the secret holding one server's environment and OAuth values:
/// the full name is this prefix plus the install identifier.
pub const ENV_SECRET_PREFIX: &str = "mcp_client_env/";
/// The secret holding the key the tool-cache fingerprints are keyed with.
pub const CACHE_KEY_SECRET: &str = "mcp_cache_fingerprint_key";

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

/// The module's storage grant.
#[must_use]
pub fn grant() -> StorageGrant {
    StorageGrant {
        collections: collection_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect(),
        // Index search reads the source's rows and filters by substring, so no
        // collection needs full-text indexing from the backend.
        searchable: Vec::new(),
        streams: vec![AUDIT_STREAM.to_string()],
        secrets: vec![
            format!("{ENV_SECRET_PREFIX}*"),
            CACHE_KEY_SECRET.to_string(),
        ],
    }
}

/// The collections the module declares, with their indexes.
#[must_use]
pub fn collection_specs() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(SERVERS)
            .index(IndexSpec::new("qualified_name", ["qualified_name"]))
            .index(IndexSpec::new("installed_at", ["installed_at"])),
        CollectionSpec::new(SERVER_NAMES),
        CollectionSpec::new(REGISTRY_CACHE).index(IndexSpec::new("cache_key", ["cache_key"])),
        CollectionSpec::new(TOOL_CACHE),
        CollectionSpec::new(REGISTRY_INDEX).index(IndexSpec::new("source", ["source"])),
        CollectionSpec::new(REGISTRY_INDEX_STATE),
    ]
}
