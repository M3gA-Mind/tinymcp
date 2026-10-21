//! The MCP module's pure storage declarations.
//!
//! This contract lists the collection shapes and stable names that a host may
//! provision. Migration execution and storage behavior live in `tinymcp`.

mod schema;
mod legacy;

pub use legacy::{
    LegacyServerBody, LegacyServerDocument, LegacyServerNameClaim, ServerDocument,
    ServerDocumentBody, ServerNameClaim, convert_legacy_server,
    convert_legacy_server_name_claim, legacy_audit_stream_target,
};

pub use schema::{
    AUDIT_STREAM, MIGRATIONS, OWNER, REGISTRY_CACHE, REGISTRY_INDEX, REGISTRY_INDEX_STATE,
    SCHEMA_VERSION, SERVER_NAMES, SERVERS, TOOL_CACHE, collection_specs, doc_id, write_options,
};

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
