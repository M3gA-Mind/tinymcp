//! Storage over the host's `ai.tinyhumans.tinystorage.Storage` bus objects.
//!
//! A loaded module cannot reach the host's storage handles, so it asks the
//! host for them over its own `TinyBus` connection. The host binds the caller
//! to one scope it chooses from the broker-authenticated sender; the module
//! never names a tenant, a collection outside its [`grant`](super::grant) or a
//! backend. What comes back implements the same port traits as an in-process
//! backend, so the stores above this file are identical in both cases.

use std::path::Path;
use std::sync::Arc;

use crate::storage::types::SecretStore;
use tinybus::{BusName, Connection};
use tinystoragedrivers_api::Storage;
use tinystoragedrivers_busproxy::{connect_storage, connect_storage_at};

use super::{StoragePorts, StorageProvider};
use crate::error::{Error, Result};

/// A provider whose ports are the host's, reached over the bus.
///
/// One bus connection serves one scope, so every data directory answers the
/// same ports: the host, not the path, decides whose state this is.
#[derive(Debug, Clone)]
pub struct BusProvider {
    ports: StoragePorts,
}

impl BusProvider {
    /// Connects to the explicit host callback that was provisioned for this
    /// module instance.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the host serves no storage, refuses the
    /// capability calls, or the bus fails.
    pub async fn connect_at(connection: Connection, destination: BusName) -> Result<Self> {
        Self::connect_at_with(connection, destination, None).await
    }

    async fn connect_at_with(
        connection: Connection,
        destination: BusName,
        secrets: Option<Arc<dyn SecretStore>>,
    ) -> Result<Self> {
        Self::from_storage_result(connect_storage_at(connection, destination).await, secrets)
    }

    fn from_storage_result(
        storage: tinystoragedrivers_types::Result<Storage>,
        secrets: Option<Arc<dyn SecretStore>>,
    ) -> Result<Self> {
        let storage =
            storage.map_err(|source| Error::store("connecting to the host storage", source))?;
        Ok(Self::from_storage(&storage, secrets))
    }

    fn from_storage(storage: &Storage, secrets: Option<Arc<dyn SecretStore>>) -> Self {
        let secrets = secrets.unwrap_or_else(|| Arc::clone(storage.secrets()));
        Self {
            ports: StoragePorts::from_storage(storage, secrets),
        }
    }

    /// Connects to the storage service the host serves under its well-known
    /// name. The host, not this module, binds the scope from the
    /// broker-authenticated sender and enforces the grant, as with a private
    /// callback; the difference is only that every module reaches the one
    /// service rather than an instance-bound destination.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when no peer serves the storage objects or the
    /// host refuses the capability calls. There is no memory fallback.
    pub async fn connect_service(connection: Connection) -> Result<Self> {
        Self::connect_ordinary_with(connection, None).await
    }

    /// Ordinary, non-attested bus connection with explicit `secrets`; the
    /// in-process port tests supply their own secret store.
    pub(crate) async fn connect_ordinary_with(
        connection: Connection,
        secrets: Option<Arc<dyn SecretStore>>,
    ) -> Result<Self> {
        let storage = connect_storage(connection)
            .await
            .map_err(|source| Error::store("connecting to the host storage", source))?;
        Ok(Self::from_storage(&storage, secrets))
    }

    /// A provider over already-connected `ports`.
    #[must_use]
    pub fn from_ports(ports: StoragePorts) -> Self {
        Self { ports }
    }
}

#[cfg(test)]
#[path = "bus_unit_tests.rs"]
mod tests;

impl StorageProvider for BusProvider {
    fn ports_for(&self, _data_dir: &Path) -> Result<StoragePorts> {
        Ok(self.ports.clone())
    }

    fn default_ports(&self) -> Result<StoragePorts> {
        Ok(self.ports.clone())
    }
}
