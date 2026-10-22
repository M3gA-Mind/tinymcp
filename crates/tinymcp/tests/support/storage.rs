//! Public-API storage fixtures for integration tests.

use std::sync::Arc;

use tinymcp::registry::Store;
use tinymcp::storage::StoragePorts;
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::Scope;

/// Opens a store backed by fresh in-memory document, stream, and secret ports.
pub(crate) fn open_memory_store() -> Store {
    let backend = MemoryBackend::new();
    let scope = Scope::new("integration-test").expect("valid test scope");
    let storage = backend
        .for_scope(&scope)
        .expect("open in-memory storage");
    let ports = StoragePorts::from_storage(&storage, Arc::clone(storage.secrets()));
    Store::open(ports).expect("open in-memory store")
}
