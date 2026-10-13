//! What each member of the interface returns.
//!
//! One type per operation whose reply is more than a single value already
//! modelled elsewhere. Operations that answer with a plain
//! [`Vec<InstalledServer>`](crate::InstalledServer) or a
//! [`RegistryServerDetail`](crate::RegistryServerDetail) use that type directly
//! rather than wrapping it in a struct with one field.
//!
//! # These are replies, not envelopes
//!
//! There is no shared success-or-error wrapper. A failure crosses the bus as a
//! failure; these types describe what a *successful* call produced. That keeps
//! a caller from having to unwrap two layers to find out whether anything went
//! wrong, and keeps the failure path typed rather than stringly.

mod explain;
mod types;

pub use explain::{
    NotConnectedExplanation, NotConnectedRemedy, explain_not_connected, not_connected_message,
    require_identifier,
};

pub use types::{
    ConnectOutcome, InstallOutcome, RegistryFreshness, RegistrySearchPage, RegistrySettings,
    SearchCuration, ServerDetail, ToolCallOutcome, UpdateEnvOutcome, UpdateEnvStatus,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod test;

#[cfg(test)]
#[path = "explain_tests.rs"]
mod explain_test;
