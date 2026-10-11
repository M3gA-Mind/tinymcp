//! The persistent tool cache: what each server advertised the last time it was
//! listed.
//!
//! A host wants a server's tools *before* the server is reachable — at boot,
//! on a resumed conversation, on a turn that must not wait for a subprocess to
//! spawn. This table is that answer. It is written whenever a listing
//! succeeds and read without touching the network.
//!
//! # A cache, not an authorization
//!
//! A row says what a server offered, not that it may be called. Calling still
//! goes through a live connection, which re-checks everything. That is why a
//! disconnect keeps the row and an uninstall or a disable removes it: the
//! first is a state that will change back, the other two are the user saying
//! the server's tools should stop appearing.
//!
//! # Keyed by a fingerprint of the definition
//!
//! Each row carries a keyed digest of the server definition. The input includes
//! credential values so rotation invalidates cached tools, while the per-store
//! key prevents the database digest from becoming an offline credential oracle.
//! A read whose fingerprint differs misses, so editing a server's definition
//! can never surface the old server's tools under the new one.

use rusqlite::{OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};

use super::types::{Store, now_ms};
use crate::error::{Error, Result};
use tinymcp_bus::{InstalledServer, McpTool};

/// The key prefix for a server pinned in a host's configuration, keeping it
/// out of the installed servers' identifier space.
const STATIC_KEY_PREFIX: &str = "static:";

/// A server's tools as last listed.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedTools {
    /// The tools, as the server advertised them.
    pub tools: Vec<McpTool>,
    /// When they were listed, in Unix epoch milliseconds.
    pub cached_at: i64,
}

/// The cache key for a server pinned in a host's configuration.
#[must_use]
pub fn static_cache_key(name: &str) -> String {
    format!("{STATIC_KEY_PREFIX}{}", name.trim())
}

/// The fingerprint of an installed server's definition.
///
/// Covers connection identity and configuration, including credential values.
/// The raw digest is used only as input to the per-store keyed cache identity;
/// it is never persisted directly.
#[must_use]
pub fn installed_fingerprint(server: &InstalledServer) -> String {
    let mut env_keys = server.env_keys.clone();
    env_keys.sort();
    fingerprint(&[
        server.transport.dispatch_kind(),
        server.transport.deployment_url().unwrap_or_default(),
        server.command_kind.as_str(),
        &server.command,
        &server.args.join("\0"),
        &env_keys.join("\0"),
    ])
}

/// A hex SHA-256 over `parts`, each length-prefixed so no two different part
/// lists can produce the same input.
pub(crate) fn fingerprint(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hex(&hasher.finalize())
}

/// A keyed cache identity, so the database does not become an offline oracle
/// for low-entropy credentials that participate in the caller's definition
/// fingerprint.
pub(super) fn keyed_fingerprint(key: &[u8; 32], value: &str) -> String {
    use sha2::{Digest as _, Sha256};

    const BLOCK_BYTES: usize = 64;
    let mut inner_pad = [0x36; BLOCK_BYTES];
    let mut outer_pad = [0x5c; BLOCK_BYTES];
    for (index, byte) in key.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(value.as_bytes());
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    hex(&outer.finalize())
}

/// Lower-case hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

impl Store {
    /// Records `tools` as what the server under `server_key` advertises now.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the write fails.
    pub fn put_cached_tools(
        &self,
        server_key: &str,
        fingerprint: &str,
        tools: &[McpTool],
    ) -> Result<()> {
        let encoded = serde_json::to_string(tools)?;
        let fingerprint = self.keyed_cache_fingerprint(fingerprint);
        self.connection
            .lock()
            .execute(
                "INSERT OR REPLACE INTO mcp_tool_cache (server_key, fingerprint, tools_json, cached_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![server_key, fingerprint, encoded, now_ms()],
            )
            .map_err(|source| Error::store("writing the tool cache", source))?;
        tracing::debug!(server_key, tools = tools.len(), "cached a tool listing");
        Ok(())
    }

    /// The tools last cached for `server_key`, when they were cached for the
    /// same definition.
    ///
    /// A row whose fingerprint differs is treated as absent: it describes a
    /// server that no longer exists under this key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the query fails or the row cannot be
    /// decoded.
    pub fn cached_tools(&self, server_key: &str, fingerprint: &str) -> Result<Option<CachedTools>> {
        let fingerprint = self.keyed_cache_fingerprint(fingerprint);
        let row: Option<(String, String, i64)> = self
            .connection
            .lock()
            .query_row(
                "SELECT fingerprint, tools_json, cached_at FROM mcp_tool_cache WHERE server_key = ?1",
                params![server_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|source| Error::store("reading the tool cache", source))?;

        let Some((stored, tools_json, cached_at)) = row else {
            return Ok(None);
        };
        if stored != fingerprint {
            tracing::debug!(server_key, "the cached tools describe another definition");
            return Ok(None);
        }
        Ok(Some(CachedTools {
            tools: serde_json::from_str(&tools_json)?,
            cached_at,
        }))
    }

    /// Drops the cached tools for `server_key`, reporting whether a row went.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the delete fails.
    pub fn forget_cached_tools(&self, server_key: &str) -> Result<bool> {
        let removed = self
            .connection
            .lock()
            .execute(
                "DELETE FROM mcp_tool_cache WHERE server_key = ?1",
                params![server_key],
            )
            .map_err(|source| Error::store("forgetting cached tools", source))?;
        Ok(removed > 0)
    }
}
