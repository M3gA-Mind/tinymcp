//! Configured-client lifecycle: the pure, I/O-free half of a host's MCP client.
//!
//! `OpenConfigured`, `Close` and `TransitionStatus` are asynchronous receipts:
//! the module accepts a call, a caller can be cancelled or lose the reply, and
//! cleanup is observed by polling. Every host would otherwise re-derive the
//! same bookkeeping, so it lives here once, as data with no I/O and no runtime:
//!
//! * [`ConfiguredLifecycle`] is the per-directory state machine. It retains the
//!   open and close receipts before polling, so a later call resumes cleanup
//!   after its caller was cancelled. The driver performs the bus call each
//!   [`Step`] names and feeds the reply back.
//! * [`ClientTable`] is the process-wide cache: admission keyed by directory,
//!   retirement of stale clients, and receipt-based cleanup reservations that
//!   keep a directory unavailable until the module acknowledges its close.
//!
//! The host decides which configuration fields matter (it builds the
//! [`McpClientConfig`](crate::McpClientConfig) it passes in) and owns approval,
//! audit and error mapping; this module decides nothing about policy.

mod machine;
mod table;

pub use machine::{ConfiguredLifecycle, Fault, OpenObject, Poll, Step};
pub use table::{AdmitError, CleanupAction, CleanupReservation, ClientTable, Retire};

#[cfg(test)]
#[path = "machine_tests.rs"]
mod machine_tests;
#[cfg(test)]
#[path = "table_tests.rs"]
mod table_tests;
