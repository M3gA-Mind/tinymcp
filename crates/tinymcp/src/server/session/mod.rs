//! Per-session client provenance.
//!
//! A host attributes what a client does — a memory write, an audit row — to
//! *which* client did it. MCP's only statement of that is `clientInfo.name` in
//! `initialize`, which is free text, so a session normalizes it into a stable
//! slug under a host-chosen prefix: `Claude Desktop` becomes
//! `mcp:claude-desktop`.
//!
//! The first `initialize` a session sees decides the value for good. A client
//! that re-initializes cannot rename itself, and one that first initialized
//! anonymously stays anonymous — attribution that could change mid-session
//! would be attribution nobody could rely on.

use serde_json::Value;

/// The provenance of one client connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    prefix: String,
    client_source_type: Option<String>,
    initialized: bool,
    protocol_version: Option<String>,
}

impl ClientSession {
    /// A session whose source type is `prefix` until `initialize` names the
    /// client.
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            client_source_type: None,
            initialized: false,
            protocol_version: None,
        }
    }

    /// Records the version selected by the latest successful initialize.
    pub(crate) fn observe_initialize_response(&mut self, protocol_version: &str) {
        self.initialized = true;
        self.protocol_version = Some(protocol_version.to_owned());
    }

    /// Returns the module-owned transport metadata for this protocol session.
    #[must_use]
    pub(crate) fn metadata(&self) -> tinymcp_bus::ServerSessionMetadata {
        tinymcp_bus::ServerSessionMetadata {
            initialized: self.initialized,
            protocol_version: self.protocol_version.clone(),
        }
    }

    /// Records the client named in `initialize` params, if this is the
    /// session's first `initialize`.
    ///
    /// Without a usable `clientInfo.name` the bare prefix is locked in.
    pub fn observe_initialize_params(&mut self, params: &Value) {
        if let Some(client_source_type) = self.client_source_type.as_deref() {
            tracing::trace!(
                "[mcp_server] initialize provenance already captured client_source_type={} param_keys={:?}",
                client_source_type,
                object_keys(params)
            );
            return;
        }

        let Some(normalized_name) = params
            .get("clientInfo")
            .and_then(|client_info| client_info.get("name"))
            .and_then(Value::as_str)
            .and_then(Self::normalize_client_name)
        else {
            tracing::trace!(
                "[mcp_server] initialize provenance fallback locked client_source_type={} param_keys={:?}",
                self.prefix,
                object_keys(params)
            );
            self.client_source_type = Some(self.prefix.clone());
            return;
        };

        let client_source_type = format!("{}:{normalized_name}", self.prefix);
        tracing::debug!(
            "[mcp_server] initialize provenance captured base_source_type={} normalized_client_name={} client_source_type={}",
            self.prefix,
            normalized_name,
            client_source_type
        );
        self.client_source_type = Some(client_source_type);
    }

    /// The session's source type: `<prefix>:<client>` once a client has been
    /// named, otherwise the bare prefix.
    #[must_use]
    pub fn source_type(&self) -> &str {
        self.client_source_type.as_deref().unwrap_or(&self.prefix)
    }

    /// Reduces a client name to a lowercase ASCII slug, or `None` when nothing
    /// alphanumeric survives.
    ///
    /// Runs of anything other than ASCII letters and digits collapse to a
    /// single `-`, with none at either end.
    #[must_use]
    pub fn normalize_client_name(raw: &str) -> Option<String> {
        let mut normalized = String::new();
        let mut previous_was_separator = false;

        for ch in raw.trim().chars() {
            if ch.is_ascii_alphanumeric() {
                normalized.push(ch.to_ascii_lowercase());
                previous_was_separator = false;
            } else if !normalized.is_empty() && !previous_was_separator {
                normalized.push('-');
                previous_was_separator = true;
            }
        }

        while normalized.ends_with('-') {
            normalized.pop();
        }

        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }
}

/// The sorted keys of `value` when it is an object, for diagnostics.
pub(crate) fn object_keys(value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod test;
