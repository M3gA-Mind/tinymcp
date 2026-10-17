//! The module's storage declaration: [`storage_schema`], its v1 to v2
//! migration steps, and the names they share.
//!
//! This lives in the contract crate so a host composes the schema and
//! generates the module's storage grant without linking the implementation. It
//! needs only the storage *ports* (`tinystoragedrivers-core`, no driver, no
//! I/O of its own); every step is a pure function of the records it is handed.

mod migrate;
mod schema;

pub use schema::{
    AUDIT_STREAM, OWNER, REGISTRY_CACHE, REGISTRY_INDEX, REGISTRY_INDEX_STATE, SCHEMA_VERSION,
    SERVER_NAMES, SERVERS, TOOL_CACHE, doc_id, storage_schema, write_options,
};
