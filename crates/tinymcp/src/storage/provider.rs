//! Choosing the ports for a data directory.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use tinystoragedrivers_core::MemoryStorage;

use super::StoragePorts;
use crate::error::Result;

/// Hands the module its storage ports, one set per data directory.
///
/// A host that serves several directories (profiles, tenants) maps each to
/// its own scope; the module only asks.
pub trait StorageProvider: Send + Sync + std::fmt::Debug {
    /// The ports for `data_dir`. Calling it again for the same directory
    /// answers the same state.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Store`] when the directory has no storage.
    fn ports_for(&self, data_dir: &Path) -> Result<StoragePorts>;
}

/// An in-memory provider: one backend, one scope per directory.
///
/// State lives as long as the provider, so reopening a directory in the same
/// process finds what was written. Nothing is written to disk.
#[derive(Debug, Default)]
pub struct MemoryProvider {
    backend: MemoryStorage,
    opened: Mutex<HashMap<String, StoragePorts>>,
}

impl MemoryProvider {
    /// An empty provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl StorageProvider for MemoryProvider {
    fn ports_for(&self, data_dir: &Path) -> Result<StoragePorts> {
        let key = data_dir.to_string_lossy().into_owned();
        let mut opened = self.opened.lock();
        if let Some(ports) = opened.get(&key) {
            return Ok(ports.clone());
        }
        let scope = format!("dir-{}", crate::registry::store::hex_digest(key.as_bytes()));
        let ports = StoragePorts::memory_scoped(&self.backend, &scope)?;
        opened.insert(key, ports.clone());
        Ok(ports)
    }
}

/// The slot holding the provider services are built with.
#[derive(Debug)]
pub(crate) struct ProviderSlot(Mutex<Arc<dyn StorageProvider>>);

impl ProviderSlot {
    /// A slot holding a fresh in-memory provider.
    pub(crate) fn new() -> Self {
        Self(Mutex::new(Arc::new(MemoryProvider::new())))
    }

    /// Replaces the provider.
    pub(crate) fn install(&self, provider: Arc<dyn StorageProvider>) {
        *self.0.lock() = provider;
    }

    /// The current provider.
    pub(crate) fn current(&self) -> Arc<dyn StorageProvider> {
        Arc::clone(&self.0.lock())
    }
}

fn slot() -> &'static ProviderSlot {
    static INSTALLED: OnceLock<ProviderSlot> = OnceLock::new();
    INSTALLED.get_or_init(ProviderSlot::new)
}

/// Makes `provider` the process's storage, for every service built afterwards.
///
/// A host calls this once at startup, before building a service.
pub fn install_provider(provider: Arc<dyn StorageProvider>) {
    slot().install(provider);
}

/// The provider services are built with: the installed one, or the
/// process-wide in-memory provider.
#[must_use]
pub fn installed_provider() -> Arc<dyn StorageProvider> {
    slot().current()
}
