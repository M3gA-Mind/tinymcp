//! The bus provider over a real in-process broker: one host connection serving
//! the storage objects, one module connection calling them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use tinybus::{Connection, broker::Broker, transport::memory::MemoryBus};
use tinystoragedrivers_busproxy::{PerCallerEndpoint, serve};
use tinystoragedrivers_client::StorageServer;
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};
use tinystoragedrivers_secrets::MemorySecrets;

use super::*;
use crate::Store;

/// A host serving `backend`'s `scope` to every caller, with secrets.
async fn host(backend: &MemoryStorage, scope: &str) -> (MemoryBus, Connection) {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let host = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let storage = backend.for_scope(&Scope::new(scope).unwrap()).unwrap();
    let secrets = Arc::new(MemorySecrets::new());
    let endpoint = PerCallerEndpoint::new(move |_sender| {
        Some(StorageServer::new(storage.clone()).with_secrets(secrets.clone()))
    });
    serve(&host, Arc::new(endpoint)).await.unwrap();
    (bus, host)
}

async fn module(bus: &MemoryBus) -> BusProvider {
    let connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    BusProvider::connect(connection).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn installed_servers_and_their_secrets_survive_reopening_over_the_bus() {
    let backend = MemoryStorage::new();
    let (bus, _host) = host(&backend, "module-tinymcp").await;
    let dir = std::env::temp_dir().join("tinymcp-bus-test");

    let first = module(&bus).await.ports_for(&dir).unwrap();
    first.prepare().unwrap();
    let store = Store::open(first).unwrap();
    assert!(!store.ports().has_state().unwrap());
    store
        .insert_server(&crate::registry::store::testing_server("srv-1"))
        .unwrap();
    store
        .set_env_values(
            "srv-1",
            &std::collections::BTreeMap::from([("TOKEN".to_owned(), "s3cret".to_owned())]),
        )
        .unwrap();

    // A second module connection (a restart) sees the same state.
    let second = Store::open(module(&bus).await.ports_for(&dir).unwrap()).unwrap();
    assert_eq!(second.list_servers().unwrap().len(), 1);
    assert_eq!(
        second.load_env_values("srv-1").unwrap().get("TOKEN").map(String::as_str),
        Some("s3cret")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_directory_answers_the_one_host_bound_scope() {
    let backend = MemoryStorage::new();
    let (bus, _host) = host(&backend, "module-tinymcp").await;
    let provider = module(&bus).await;
    let a = provider.ports_for(&std::env::temp_dir().join("a")).unwrap();
    a.prepare().unwrap();
    Store::open(a)
        .unwrap()
        .insert_server(&crate::registry::store::testing_server("srv-1"))
        .unwrap();
    let b = provider.ports_for(&std::env::temp_dir().join("b")).unwrap();
    assert!(b.has_state().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn connecting_without_a_host_is_an_error_not_a_silent_memory_store() {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let lonely = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    assert!(BusProvider::connect(lonely).await.is_err());
}
