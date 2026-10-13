//! Storage over the host's `ai.tinyhumans.tinystorage.Storage` bus objects.
//!
//! A loaded module cannot reach the host's storage handles, so it asks the
//! host for them over its own `TinyBus` connection. The host binds the caller
//! to one scope it chooses from the broker-authenticated sender; the module
//! never names a tenant, a collection outside its [`grant`](super::grant) or a
//! backend. What comes back implements the same port traits as an in-process
//! backend, so the stores above this file are identical in both cases.

use std::path::Path;

use tinybus::Connection;
use tinystoragedrivers_busproxy::{connect_storage, module_scope_label};
use tinystoragedrivers_core::StorageBackend;

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
    /// Connects to the host's storage objects and binds the granted scope.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the host serves no storage, refuses the
    /// capability calls, or the bus fails.
    pub async fn connect(connection: Connection) -> Result<Self> {
        let storage = connect_storage(connection)
            .await
            .map_err(|source| Error::store("connecting to the host storage", source))?;
        let scoped = storage
            .for_scope(&module_scope_label())
            .map_err(|source| Error::store("opening the host-bound scope", source))?;
        let ports = StoragePorts::from_scoped(&scoped, storage.secrets());
        Ok(Self { ports })
    }

    /// A provider over already-connected `ports`.
    #[must_use]
    pub fn from_ports(ports: StoragePorts) -> Self {
        Self { ports }
    }
}

impl StorageProvider for BusProvider {
    fn ports_for(&self, _data_dir: &Path) -> Result<StoragePorts> {
        Ok(self.ports.clone())
    }
}
