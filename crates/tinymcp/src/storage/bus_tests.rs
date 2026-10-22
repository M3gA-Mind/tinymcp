//! The bus provider over a real in-process broker: one host connection serving
//! the storage objects, one module connection calling them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use tinybus::{BusName, Connection, broker::Broker, transport::memory::MemoryBus};
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_secrets::{MemorySecrets, SecretStore};
use tinystoragedrivers_testkit::MemoryBackend as MemoryStorage;
use tinystoragedrivers_types::{CollectionSpec, Precondition, Scope, WriteOptions};

use super::test_host::{TestHost, serve_storage};
use super::*;
use crate::Store;

async fn host(backend: &MemoryStorage, scope: &str) -> (MemoryBus, TestHost) {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let host = serve_storage(&bus, backend, scope).await;
    (bus, host)
}

fn a_server(id: &str) -> tinymcp_bus::InstalledServer {
    tinymcp_bus::InstalledServer {
        server_id: id.to_string(),
        qualified_name: "@test/server".to_string(),
        display_name: "Test Server".to_string(),
        description: None,
        icon_url: None,
        command_kind: tinymcp_bus::CommandKind::Node,
        command: "npx".to_string(),
        args: vec!["-y".to_string()],
        env_keys: vec!["TOKEN".to_string()],
        config: None,
        installed_at: 1_700_000_000_000,
        last_connected_at: None,
        transport: tinymcp_bus::Transport::Stdio,
        enabled: true,
    }
}

/// A module connection. Secrets are held in `secrets`, shared across
/// connections the way the host's one store is.
async fn module(bus: &MemoryBus, secrets: &Arc<MemorySecrets>) -> BusProvider {
    let connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    BusProvider::connect_ordinary_with(connection, Some(secrets.clone() as Arc<dyn SecretStore>))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn installed_servers_and_their_secrets_survive_reopening_over_the_bus() {
    let backend = MemoryStorage::new();
    let (bus, _host) = host(&backend, "module-tinymcp").await;
    let dir = std::env::temp_dir().join("tinymcp-bus-test");

    let secrets = Arc::new(MemorySecrets::new());
    let first = module(&bus, &secrets).await.ports_for(&dir).unwrap();
    first.prepare().unwrap();
    let store = Store::open(first).unwrap();
    assert!(!store.ports().has_state().unwrap());
    store.insert_server(&a_server("srv-1")).unwrap();
    store
        .set_env_values(
            "srv-1",
            &std::collections::BTreeMap::from([("TOKEN".to_owned(), "s3cret".to_owned())]),
        )
        .unwrap();

    // A second module connection (a restart) sees the same state.
    let second = Store::open(module(&bus, &secrets).await.ports_for(&dir).unwrap()).unwrap();
    assert_eq!(second.list_servers().unwrap().len(), 1);
    assert_eq!(
        second
            .load_env_values("srv-1")
            .unwrap()
            .get("TOKEN")
            .map(String::as_str),
        Some("s3cret")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_directory_answers_the_one_host_bound_scope() {
    let backend = MemoryStorage::new();
    let (bus, _host) = host(&backend, "module-tinymcp").await;
    let secrets = Arc::new(MemorySecrets::new());
    let provider = module(&bus, &secrets).await;
    let a = provider.ports_for(&std::env::temp_dir().join("a")).unwrap();
    a.prepare().unwrap();
    Store::open(a)
        .unwrap()
        .insert_server(&a_server("srv-1"))
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
    assert!(
        BusProvider::connect_ordinary_with(lonely, None)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn callback_connection_requires_a_live_admitted_host_capability() {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let destination = BusName::new("org.example.Storage").unwrap();
    let error = BusProvider::connect_at(connection, destination)
        .await
        .expect_err("ordinary peers cannot impersonate a provisioned host callback");
    assert!(matches!(error, crate::Error::Store { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ordinary_unattested_connection_cannot_use_the_secret_store() {
    let backend = MemoryStorage::new();
    let (bus, _host) = host(&backend, "module-tinymcp").await;
    let connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let provider = BusProvider::connect_ordinary_with(connection, None)
        .await
        .unwrap();
    let ports = provider.ports_for(&std::env::temp_dir().join("c")).unwrap();
    ports.prepare().unwrap();
    // An in-process broker cannot attest the caller, so the confidential
    // secret route is refused rather than silently served.
    let refused = Store::open(ports).unwrap_err();
    assert!(
        matches!(
            &refused,
            crate::Error::Store { source, .. }
                if matches!(
                    source.kind(),
                    tinystoragedrivers_types::ErrorKind::Unsupported(_)
                )
        ),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_over_ready_ports_hands_out_clones_of_them() {
    let backend = MemoryStorage::new();
    let (bus, _host) = host(&backend, "module-tinymcp").await;
    let secrets = Arc::new(MemorySecrets::new());
    let ports = module(&bus, &secrets)
        .await
        .ports_for(&std::env::temp_dir().join("d"))
        .unwrap();
    let provider = BusProvider::from_ports(ports);
    let again = provider.ports_for(&std::env::temp_dir().join("e")).unwrap();
    again.prepare().unwrap();
    assert!(!again.has_state().unwrap());
    assert!(format!("{provider:?}").contains("BusProvider"));
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_migration_omits_host_only_creation_time_on_the_bus() {
    let backend = MemoryStorage::new();
    let storage = backend
        .for_scope(&Scope::new("module-tinymcp").unwrap())
        .unwrap();
    let local_docs = Arc::clone(storage.documents());
    local_docs
        .ensure_collection(&CollectionSpec::new("flows.items"))
        .await
        .unwrap();
    local_docs
        .put_with(
            "flows.items",
            "one",
            json!({"value": "before"}),
            Precondition::Absent,
            WriteOptions::new().schema(1).created_ms(123),
        )
        .await
        .unwrap();

    let (bus, _host) = host(&backend, "module-tinymcp").await;
    let secrets = Arc::new(MemorySecrets::new());
    let ports = module(&bus, &secrets)
        .await
        .ports_for(&std::env::temp_dir().join("schema-created-ms"))
        .unwrap();
    let before = ports
        .documents
        .get("flows.items", "one")
        .await
        .unwrap()
        .unwrap();
    let schema =
        crate::storage::local::Schema::new(crate::storage::local::Owner::new("flows").unwrap())
            .collection_with(
                2,
                CollectionSpec::new("flows.items"),
                crate::storage::local::ScopeClass::Tenant,
            )
            .step(crate::storage::local::Step::new(
                "items_v2",
                "flows.items",
                1,
                2,
                |rows| {
                    Ok(rows
                        .into_iter()
                        .map(|row| {
                            crate::storage::local::TargetRecord::new(
                                "flows.items",
                                row.id,
                                json!({"value": "after"}),
                            )
                        })
                        .collect())
                },
            ));

    schema
        .migrate(
            &ports.documents,
            &crate::storage::local::MigrateOptions::default(),
        )
        .await
        .unwrap();
    let after = ports
        .documents
        .get("flows.items", "one")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.doc["value"], "after");
    assert_eq!(after.meta.schema, 2);
    assert_eq!(after.meta.created_ms, before.meta.created_ms);
}
