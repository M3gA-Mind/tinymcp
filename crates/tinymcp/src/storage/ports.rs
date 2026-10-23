//! The handles the module works through, and the bridge to its synchronous API.

use std::future::Future;
use std::sync::{Arc, OnceLock};

#[cfg(test)]
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_api::{DocumentStore, SecretStore, Storage, StreamStore};
use tinystoragedrivers_types::{ErrorKind, Filter, Result as StorageResult};

use super::grant::LEGACY_COLLECTIONS;
use super::local::{Blocking, MigrateOptions, Migrations};

use super::schema::SERVERS;
use crate::error::{Error, Result};

/// The three ports the module persists through.
///
/// Cheap to clone: every field is a shared handle. The module's stores are
/// synchronous, so each call runs its port future on one shared
/// blocking bridge thread.
#[derive(Clone)]
pub struct StoragePorts {
    pub(crate) documents: Arc<dyn DocumentStore>,
    pub(crate) streams: Arc<dyn StreamStore>,
    pub(crate) secrets: Arc<dyn SecretStore>,
    /// Whether the host owns the schema: it declared the collections when it
    /// bound this module's scope, and its grant is the allowlist, so the
    /// module neither migrates data nor touches the shared progress ledger.
    host_managed: bool,
}

impl std::fmt::Debug for StoragePorts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoragePorts")
            .finish_non_exhaustive()
    }
}

/// The handles a port future owns while it runs on the bridge thread.
pub(crate) type Handles = StoragePorts;

impl StoragePorts {
    /// Wraps already-scoped handles.
    #[must_use]
    pub fn new(
        documents: Arc<dyn DocumentStore>,
        streams: Arc<dyn StreamStore>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            documents,
            streams,
            secrets,
            host_managed: false,
        }
    }

    /// These ports are a host's, reached over the bus: the host owns the
    /// schema ([`prepare`](Self::prepare) only checks it) and its grant does
    /// not include the migration ledger or the v1 source collections.
    #[must_use]
    pub(crate) fn host_managed(mut self) -> Self {
        self.host_managed = true;
        self
    }

    /// Uses a scoped backend handle for documents and streams, and `secrets`
    /// for secret values.
    #[must_use]
    pub fn from_storage(scoped: &Storage, secrets: Arc<dyn SecretStore>) -> Self {
        Self::new(
            Arc::clone(scoped.documents()),
            Arc::clone(scoped.streams()),
            secrets,
        )
    }

    /// A fresh, private in-memory backend: nothing survives the handles.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the scope cannot be opened.
    #[cfg(test)]
    pub(crate) fn memory() -> Result<Self> {
        Self::memory_scoped(&tinystoragedrivers_testkit::MemoryBackend::new(), "local")
    }

    /// The `scope` of a shared in-memory backend.
    ///
    /// Each call attaches a fresh secret store: keep and clone the returned
    /// ports (as `MemoryProvider` does) rather than reopening a scope, or the
    /// credentials and cache key written through the first open are not seen.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the scope is invalid.
    #[cfg(test)]
    pub(crate) fn memory_scoped(
        backend: &tinystoragedrivers_testkit::MemoryBackend,
        scope: &str,
    ) -> Result<Self> {
        let scope = tinystoragedrivers_types::Scope::new(scope)
            .map_err(|source| Error::store("opening a scope", source))?;
        let scoped = backend
            .for_scope(&scope)
            .map_err(|source| Error::store("opening a scope", source))?;
        Ok(Self::from_storage(
            &scoped,
            Arc::new(tinystoragedrivers_secrets::MemorySecrets::new()),
        ))
    }

    /// Asks the documents port for the collections [`grant`](super::grant)
    /// declares. Idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the port refuses a collection.
    pub fn prepare(&self) -> Result<()> {
        if self.host_managed {
            // The host declared these when it bound the scope, so this only
            // confirms host and module agree on them. Running the migrations
            // would reach for collections the host's grant keeps out of reach.
            return self.run("declaring the collections", |ports| async move {
                for spec in tinymcp_bus::storage::collection_specs() {
                    ports.documents.ensure_collection(&spec).await?;
                }
                Ok(())
            });
        }
        self.run("declaring the collections", |ports| async move {
            let schema = super::schema::storage_schema()?;
            schema
                .migrate(&ports.documents, &MigrateOptions::default())
                .await?;
            let migration = Migrations::new(&ports.documents, super::schema::OWNER);
            super::migrate::migrate_audit_streams(&*ports.streams, &migration).await?;
            Ok(())
        })
    }

    /// Whether anything has been installed here, for a host asking whether a
    /// data directory already holds state.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the port fails.
    pub fn has_state(&self) -> Result<bool> {
        self.run("checking for stored state", |ports| async move {
            for collection in std::iter::once(SERVERS).chain(LEGACY_COLLECTIONS) {
                let count = ports
                    .documents
                    .count(collection, &Filter::default())
                    .await
                    .or_else(|error| {
                        // A collection never declared here holds nothing, and
                        // the probe must not create it; any other failure (an
                        // unavailable or refusing port) is not "no installs".
                        if error.kind() == ErrorKind::NotFound {
                            Ok(0)
                        } else {
                            Err(error)
                        }
                    })?;
                if count > 0 {
                    return Ok(true);
                }
            }
            let audited = ports.streams.len(super::schema::AUDIT_STREAM).await?;
            if audited > 0 {
                return Ok(true);
            }
            let legacy_audit = ports.streams.streams("mcp_audit").await?;
            Ok(legacy_audit
                .iter()
                .any(|name| name == "mcp_audit" || name.starts_with("mcp_audit/")))
        })
    }

    /// Runs one port future to completion and maps its failure to
    /// [`Error::Store`] under `action`.
    pub(crate) fn run<T, F>(
        &self,
        action: &'static str,
        work: impl FnOnce(Handles) -> F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = StorageResult<T>> + Send + 'static,
    {
        let future = work(self.clone());
        bridge()
            .map_err(|source| Error::store(action, source))?
            .run(future)
            .map_err(|source| Error::store(action, source))?
            .map_err(|source| Error::store(action, source))
    }
}

/// The one bridge thread every store in the process shares.
fn bridge() -> StorageResult<&'static Blocking> {
    static BRIDGE: OnceLock<Blocking> = OnceLock::new();
    if let Some(bridge) = BRIDGE.get() {
        return Ok(bridge);
    }
    let started = Blocking::new()?;
    Ok(BRIDGE.get_or_init(|| started))
}
