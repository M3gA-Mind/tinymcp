//! Results of reconciling the persisted server document.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One server the document application installed, updated, or removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedServer {
    /// The install's stable identifier.
    pub server_id: String,
    /// The document key associated with the install.
    pub name: String,
}

/// The store changes made by applying a replacement document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigApplyReport {
    /// Servers newly installed from the document.
    pub installed: Vec<AppliedServer>,
    /// Existing servers whose dial or credentials changed.
    pub updated: Vec<AppliedServer>,
    /// Servers removed because they were absent from the document.
    pub removed: Vec<AppliedServer>,
    /// Enabled installs the host may connect after returning to its caller.
    pub connect_queued: Vec<String>,
}

/// The rendered document and the reconciliation result from one apply call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigApplyResponse {
    /// The document after the module applies the requested replacement.
    pub document: Value,
    /// Changes made by the replacement.
    pub report: ConfigApplyReport,
}
