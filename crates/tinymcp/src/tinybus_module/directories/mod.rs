//! Serving more than one data directory from one loaded module.
//!
//! # The problem
//!
//! [`ModuleConfig`](super::ModuleConfig) carries one `data_dir`, supplied once
//! at load. A host whose storage moves while it runs — login rewrites the
//! active-user marker and the workspace changes from `users/local` to
//! `users/<id>`, logout reverses it — would keep being answered from the store
//! it had at load. That is worse than an error: the answer is wrong rather than
//! absent.
//!
//! # The seam
//!
//! The root object grows `Open(data_dir)` and `OpenConfigured(request)` members
//! that serve an object. `Open` keeps its original load-time client settings
//! and string response; `OpenConfigured` supplies a workspace's own server set,
//! credentials, identity and proxy together with its data directory, returning
//! an object path and opaque transition id.
//! Each object is an ordinary [`McpService`](super::McpService) over its own
//! store and client configuration. A changed configuration replaces the
//! service at the same object path after current calls finish, then stops the
//! old service's background work and transports. `TransitionStatus` observes
//! that work without waiting under `TinyBus` dispatch. `Close(data_dir)` removes
//! a child object or resets the root to an empty in-memory service; its
//! transition is complete only after module-owned cleanup finishes.
//!
//! # One object per directory
//!
//! Opening the same directory with the same client settings twice returns the
//! same path, and the root's own directory maps to the root path. `Open` keeps
//! its legacy idempotent behavior; `OpenConfigured` replaces a service when its
//! client settings change instead of returning stale credentials or endpoints.
//! At most one lifecycle operation runs at a time. While a transition is
//! pending, another mutation fails quickly and the caller can retry after
//! `TransitionStatus` reports completion. Active reservations remain counted
//! through close cleanup.
//!
//! Directories are matched by the exact absolute path the caller supplied. A
//! host should pass the same spelling each time; the module does not resolve
//! symlinks, because a path that does not exist yet has nothing to resolve.
//!
//! # Bounds
//!
//! Closed child objects become unknown paths. Their monotonically allocated
//! identifiers are never reused. [`MAX_OPEN_DIRECTORIES`] caps active and
//! closing reservations, so callers can release workspaces without allowing a
//! fast open/close loop to exhaust file descriptors.
//!
//! Each opened directory also runs its own boot pass and supervisor, so a user's
//! installed servers are connected after a login just as they were at load.

mod types;

pub(super) use types::DirectoryOpener;
#[cfg(test)]
pub(super) use types::MAX_OPEN_DIRECTORIES;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod test;
