//! Pure reads of the registry's answers that every host would otherwise copy.
//!
//! Three small rules live here instead of in each host's RPC adapter:
//! identifier validation, the JSON shape a catalog detail screen renders, and
//! how a `NotConnected` answer reads once it is set against the installs.
//! None of them performs I/O; the host still makes the calls and chooses the
//! wording of the remedy (which tool names exist on *its* surface).

use serde_json::{Value, json};

use crate::{ConnStatus, RegistryServerDetail};

use super::ServerDetail;

/// Trims an identifier and refuses a blank one, naming the field.
///
/// # Errors
///
/// Returns `"<field> must not be empty"` when nothing is left after trimming.
pub fn require_identifier(value: &str, field: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(trimmed.to_string())
}

impl ServerDetail {
    /// The catalog record as a JSON object with `required_env_keys` merged in,
    /// the single shape a registry detail screen renders.
    ///
    /// # Errors
    ///
    /// Returns the serialization failure text.
    pub fn server_json(&self) -> Result<Value, String> {
        let mut server = serde_json::to_value::<&RegistryServerDetail>(&self.server)
            .map_err(|error| format!("serialization error: {error}"))?;
        if let Some(object) = server.as_object_mut() {
            object.insert("required_env_keys".into(), json!(self.required_env_keys));
        }
        Ok(server)
    }
}

/// The tools a caller can actually reach, which decides the remedy a refusal
/// names. Naming a tool the caller cannot call is an instruction it cannot
/// follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotConnectedRemedy<'a> {
    /// The call that connects an install, when this caller has one.
    pub connect: Option<&'a str>,
    /// The call that lists installed servers; every caller has one.
    pub status: &'a str,
}

/// What a `NotConnected` answer for a requested id means, read against every
/// install.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotConnectedExplanation {
    /// The requested string is a registry qualified name installed as exactly
    /// one server: use this `server_id` instead.
    Resolved(String),
    /// A refusal that names what is actually wrong.
    Refused(String),
}

/// Reads "not connected" against the installs.
///
/// The module answers `NotConnected` for any id without a live connection,
/// including one never installed and a qualified name passed where an install's
/// `server_id` belongs. "Not connected" is true of that string and false of the
/// server, so say what the string is.
#[must_use]
pub fn explain_not_connected(
    requested: &str,
    installs: &[ConnStatus],
    remedy: NotConnectedRemedy<'_>,
) -> NotConnectedExplanation {
    if let Some(install) = installs.iter().find(|s| s.server_id == requested) {
        return NotConnectedExplanation::Refused(not_connected_message(install, remedy));
    }
    let named: Vec<&str> = installs
        .iter()
        .filter(|s| s.qualified_name == requested)
        .map(|s| s.server_id.as_str())
        .collect();
    match named.as_slice() {
        [server_id] => NotConnectedExplanation::Resolved((*server_id).to_string()),
        [] => NotConnectedExplanation::Refused(format!(
            "no installed MCP server has server_id={requested}; server_id is the install's \
             identifier, not its registry name — call {} to list installed servers",
            remedy.status
        )),
        several => NotConnectedExplanation::Refused(format!(
            "{requested} is a registry name installed as several servers; pass one server_id: {}",
            several.join(", ")
        )),
    }
}

/// An installed server with no live connection: where it stands and how to
/// connect it.
#[must_use]
pub fn not_connected_message(install: &ConnStatus, remedy: NotConnectedRemedy<'_>) -> String {
    let how = remedy.connect.map_or_else(
        || "it has to be connected before its tools can be listed".to_string(),
        |connect| format!("connect it first via {connect}"),
    );
    let base = format!(
        "server_id={} is {}; {how}",
        install.server_id,
        install.status.as_str(),
    );
    match &install.last_error {
        Some(error) => format!("{base} (last error: {error})"),
        None => base,
    }
}
