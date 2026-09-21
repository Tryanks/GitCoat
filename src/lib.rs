//! GitCoat: a read-only web UI for a single Git repository.
//!
//! The crate is a library so integration tests can build the router in-process;
//! the `gitcoat` binary is a thin wrapper around [`app::router`].

// Compiles `locales/*.yml` into the binary; English fills in missing keys.
rust_i18n::i18n!("locales", fallback = "en");

pub mod app;
pub mod config;
pub mod git;
pub mod l10n;
pub mod limits;
pub mod render;

/// Crate version shown in the footer.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
