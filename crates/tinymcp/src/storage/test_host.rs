//! A stand-in for the host's storage objects on an in-process bus.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use tinybus::{Connection, transport::memory::MemoryBus};
use tinystoragedrivers_busproxy::{PerCallerEndpoint, ServedStorage, serve};
use tinystoragedrivers_client::StorageServer;
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};
use tinystoragedrivers_secrets::MemorySecrets;

/// Keeps the served objects registered while it lives.
pub(crate) struct TestHost {
    _connection: Connection,
    _served: ServedStorage,
}

/// Serves `backend`'s `scope` (with a secret store) to every caller on `bus`.
///
/// The bus must already have a broker. A module connection started after this
/// call finds the storage objects and is bound to the one scope.
pub(crate) async fn serve_storage(
    bus: &MemoryBus,
    backend: &MemoryStorage,
    scope: &str,
) -> TestHost {
    let connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let storage = backend.for_scope(&Scope::new(scope).unwrap()).unwrap();
    let secrets = Arc::new(MemorySecrets::new());
    let endpoint = PerCallerEndpoint::new(move |_sender| {
        Some(StorageServer::new(storage.clone()).with_secrets(secrets.clone()))
    });
    let served = serve(&connection, Arc::new(endpoint)).await.unwrap();
    TestHost {
        _connection: connection,
        _served: served,
    }
}
