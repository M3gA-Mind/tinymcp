//! Installing the host-selected storage ports.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

use super::StoragePorts;
use crate::error::{Error, Result};

/// Hands the module its storage ports, one set per data directory.
pub trait StorageProvider: Send + Sync + std::fmt::Debug {
    /// Ports for `data_dir`, already bound by the host to the correct scope.
    ///
    /// # Errors
    /// Returns [`Error::Store`] when storage cannot be opened.
    fn ports_for(&self, data_dir: &Path) -> Result<StoragePorts>;

    /// Ports for a service that has no configured data directory.
    ///
    /// Hosts that support ephemeral or static-only services must provide an
    /// explicit storage binding. There is no implicit in-memory fallback.
    ///
    /// # Errors
    /// Returns [`Error::Store`] when the host has no default binding.
    fn default_ports(&self) -> Result<StoragePorts> {
        Err(Error::store(
            "opening storage without a data directory",
            crate::storage::types::StorageError::unavailable(
                "the host did not provide a default storage binding",
            ),
        ))
    }
}

/// The slot holding the host provider services are built with.
#[derive(Debug, Default)]
pub(crate) struct ProviderSlot {
    provider: Mutex<Option<Arc<dyn StorageProvider>>>,
}

impl ProviderSlot {
    /// An empty slot; the host must install its backend before service setup.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Replaces the provider.
    pub(crate) fn install(&self, provider: Arc<dyn StorageProvider>) {
        *self.provider.lock() = Some(provider);
    }

    /// Whether the host supplied a provider.
    pub(crate) fn is_installed(&self) -> bool {
        self.provider.lock().is_some()
    }

    /// The current provider, failing closed when none has been installed.
    pub(crate) fn current(&self) -> Result<Arc<dyn StorageProvider>> {
        self.provider.lock().clone().ok_or_else(|| {
            Error::store(
                "opening the host storage provider",
                crate::storage::types::StorageError::unavailable(
                    "the host has not installed storage ports",
                ),
            )
        })
    }
}

fn slot() -> &'static ProviderSlot {
    static INSTALLED: OnceLock<ProviderSlot> = OnceLock::new();
    INSTALLED.get_or_init(ProviderSlot::new)
}

/// Makes `provider` the process's storage, for every service built afterwards.
pub fn install_provider(provider: Arc<dyn StorageProvider>) {
    slot().install(provider);
}

/// Whether the host installed storage ports.
#[must_use]
pub fn provider_installed() -> bool {
    slot().is_installed()
}

/// The host provider services are built with.
///
/// # Errors
/// Returns [`Error::Store`] before the host installs its storage provider.
pub fn installed_provider() -> Result<Arc<dyn StorageProvider>> {
    slot().current()
}

#[cfg(test)]
pub(crate) use memory::MemoryProvider;

#[cfg(test)]
pub(crate) fn shared_test_provider() -> Arc<dyn StorageProvider> {
    static PROVIDER: OnceLock<Arc<MemoryProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(MemoryProvider::new()))
        .clone()
}

#[cfg(test)]
mod memory {
    //! Explicit test-only memory provider.

    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use tinystoragedrivers_testkit::MemoryBackend;

    use super::*;

    /// An in-memory provider used only by `TinyMCP`'s tests.
    #[derive(Debug, Default)]
    pub(crate) struct MemoryProvider {
        backend: MemoryBackend,
        opened: Mutex<HashMap<PathBuf, StoragePorts>>,
    }

    impl MemoryProvider {
        /// An empty provider.
        #[must_use]
        pub(crate) fn new() -> Self {
            Self::default()
        }
    }

    impl StorageProvider for MemoryProvider {
        fn ports_for(&self, data_dir: &Path) -> Result<StoragePorts> {
            if !data_dir.is_absolute() {
                return Err(crate::Error::invalid_argument(
                    "a data directory must be a non-empty absolute path",
                ));
            }
            let key = data_dir.to_path_buf();
            let mut opened = self.opened.lock();
            if let Some(ports) = opened.get(&key) {
                return Ok(ports.clone());
            }
            let label = format!("test-{}", opened.len());
            let ports = StoragePorts::memory_scoped(&self.backend, &label)?;
            opened.insert(key, ports.clone());
            Ok(ports)
        }

        fn default_ports(&self) -> Result<StoragePorts> {
            StoragePorts::memory_scoped(&self.backend, "test-default")
        }
    }
}
