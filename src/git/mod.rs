//! Read-only access to one Git repository, backed by gitoxide.
//!
//! [`types`] holds the backend-independent data model and [`Repo`] the public
//! API; the remaining modules are the gitoxide implementation.

mod commit;
mod convert;
mod diff;
mod error;
mod refs;
mod repo;
mod tree;
mod types;

#[cfg(test)]
mod tests;

pub use error::{GitError, OpenError};
pub use repo::Repo;
pub use types::*;
