//! What a host passes the module when it loads it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use tinymcp_bus::McpClientConfig;

/// The module's configuration blob.
///
/// Arrives as JSON in the loader's configuration slot. A host that configures
/// nothing supplies either the empty object or `null`, so both decode to the
/// same empty configuration. Loading the module still requires the host to
/// provision its storage callback; the module never falls back to memory.
///
/// `#[serde(default)]` covers the empty object, because it fills in absent
/// *fields*. It does not cover `null`, which is a whole document of the wrong
/// type — hence the hand-written [`Deserialize`] below. A module that refused
/// `null` would fail to load for exactly the host that asked nothing of it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModuleConfig {
    /// The host-provisioned private storage callback destination for this
    /// module instance. It is not a scope selector; the host binding chooses
    /// the scope before the module connects.
    pub storage_callback: Option<String>,
    /// Where the store and the audit log live.
    ///
    /// `None` leaves the storage root unspecified. Storage itself is supplied
    /// separately through [`ModuleConfig::storage_callback`].
    pub data_dir: Option<PathBuf>,
    /// The servers, credentials, identity, and proxy.
    pub client: McpClientConfig,
}

/// The fields as they appear on the wire.
///
/// Separate from [`ModuleConfig`] so the hand-written deserializer below can
/// derive the field handling rather than restate it.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Wire {
    storage_callback: Option<String>,
    data_dir: Option<PathBuf>,
    client: McpClientConfig,
}

impl<'de> Deserialize<'de> for ModuleConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // `Option` is what turns `null` into "nothing configured" rather than a
        // type error. Everything else decodes through the derived impl.
        let wire = Option::<Wire>::deserialize(deserializer)?.unwrap_or_default();

        Ok(Self {
            storage_callback: wire.storage_callback,
            data_dir: wire.data_dir,
            client: wire.client,
        })
    }
}
