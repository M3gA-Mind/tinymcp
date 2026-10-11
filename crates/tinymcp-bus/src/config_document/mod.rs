//! Wire payloads for reading and applying the installed-server configuration document.

mod types;

pub use types::{AppliedServer, ConfigApplyReport, ConfigApplyResponse};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
