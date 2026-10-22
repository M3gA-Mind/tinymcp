//! Where the module keeps its state: the storage ports, never a database.
//!
//! Everything this crate persists goes through three ports from
//! `tinystoragedrivers`:
//!
//! | Port | Holds |
//! | --- | --- |
//! | `DocumentStore` | installed servers, the browse cache, the tool cache, the registry index and its sync state |
//! | `StreamStore` | the append-only audit log of tool writes |
//! | `SecretStore` | each server's environment values and OAuth bundle, and the key that keys the tool-cache fingerprints |
//!
//! The crate links no database client and picks no backend. A host hands over
//! [`StoragePorts`] or installs a [`StorageProvider`] that supplies those
//! handles. Service construction fails until a host provides them; tests use
//! the TSD testkit memory backend explicitly.
//!
//! # The grant
//!
//! The module declares what it needs ([`StorageGrant`], from [`grant`]): the
//! collections and their indexes, the stream, and the secret names. A host
//! that serves storage over the bus provisions exactly that and refuses
//! anything else; the module never creates schema on its own authority beyond
//! [`StoragePorts::prepare`], which asks for the declared collections only.
//!
//! # Scope
//!
//! The module never names a tenant. The scope belongs to the handles it is
//! given, so the same code serves a desktop user and one tenant of a cloud
//! host.

#[cfg(feature = "module")]
mod bus;
#[cfg(test)]
mod failing;
mod grant;
mod local;
mod migrate;
mod migration_legacy;
mod ports;
mod provider;
mod schema;

#[cfg(feature = "module")]
pub use bus::BusProvider;
pub use grant::{CACHE_KEY_SECRET, ENV_SECRET_PREFIX, StorageGrant, collection_specs, grant};
pub use ports::StoragePorts;
pub(crate) mod types {
    //! Stable TSD v2 ports and portable value types used by MCP.
    pub(crate) use tinystoragedrivers_api::{DocumentStore, DocumentStoreExt, SecretStore};
    pub(crate) use tinystoragedrivers_types::*;
}
pub(crate) use local::Repo;
#[cfg(test)]
pub(crate) use provider::MemoryProvider;
#[cfg(test)]
pub(crate) use provider::shared_test_provider;
pub use provider::{StorageProvider, install_provider, installed_provider, provider_installed};
pub use schema::{AUDIT_STREAM, OWNER};
pub(crate) use schema::{
    REGISTRY_CACHE, REGISTRY_INDEX, REGISTRY_INDEX_STATE, SERVER_NAMES, SERVERS, TOOL_CACHE,
    doc_id, write_options,
};
#[cfg(test)]
pub(crate) use tinymcp_bus::storage::SCHEMA_VERSION;

#[cfg(all(test, feature = "module"))]
pub(crate) mod test_host;

#[cfg(all(test, feature = "module"))]
#[path = "bus_tests.rs"]
mod bus_tests;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
