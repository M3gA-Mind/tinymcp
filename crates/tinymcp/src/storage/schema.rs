//! The module's storage declaration lives in the contract crate
//! (`tinymcp_bus::storage`); this path re-exports it so the rest of the crate
//! and its callers keep one definition.

#[cfg(test)]
pub(crate) use tinymcp_bus::storage::SCHEMA_VERSION;
pub use tinymcp_bus::storage::{AUDIT_STREAM, OWNER, storage_schema};
pub(crate) use tinymcp_bus::storage::{
    REGISTRY_CACHE, REGISTRY_INDEX, REGISTRY_INDEX_STATE, SERVER_NAMES, SERVERS, TOOL_CACHE,
    doc_id, write_options,
};
