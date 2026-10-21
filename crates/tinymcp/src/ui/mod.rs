//! Tool-provided UI for hosts.
//!
//! The vocabulary and the rules (template resolution, widget document checks,
//! link classification, the widget tool-call and rendering policy) are pure,
//! so they live in the contract, [`tinymcp_bus::ui`], where a host reaches
//! them without linking this crate. This module re-exports them unchanged for
//! hosts that already take `tinymcp` as a library.

pub use tinymcp_bus::ui::*;
