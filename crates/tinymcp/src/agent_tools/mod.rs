//! Module-owned argument normalization and stable registry tool declarations.
mod arguments;
mod metadata;
pub mod naming;
pub mod schema;
pub mod scrub;
pub use arguments::normalize_tool_arguments;
pub use metadata::prepare_agent_tools;
pub use scrub::{REDACTED, SecretScrubber};
pub use tinymcp_bus::agent_tools::{
    AgentToolEffect, AgentToolSpec, ArgsError, McpCallError, McpCallOutcome, RegistryTool,
    registry_tool_specs,
};
#[cfg(test)]
#[path = "scrub_tests.rs"]
mod scrub_test;
#[cfg(test)]
#[path = "mod_tests.rs"]
mod test;
