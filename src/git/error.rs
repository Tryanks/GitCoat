//! Error types of the Git access layer.

use std::path::PathBuf;

/// Failure of a single repository query.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GitError {
    /// The caller passed a value that is not acceptable (bad ref syntax, bad path).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The requested object, ref or path does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The operation exceeded the time limit.
    #[error("repository operation timed out")]
    Timeout,
    /// The operation would exceed a size or iteration limit.
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// The repository itself is unusable (permissions, corruption, missing objects, ...).
    #[error("repository error: {0}")]
    Repo(String),
}

impl GitError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub(crate) fn limit(message: impl Into<String>) -> Self {
        Self::Limit(message.into())
    }

    pub(crate) fn repo(message: impl Into<String>) -> Self {
        Self::Repo(message.into())
    }
}

/// Failure to open the repository at startup.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The path does not exist or cannot be read.
    #[error("repository path {0} does not exist or is not a directory")]
    NotFound(PathBuf),
    /// The path is not a Git repository (neither bare nor a checkout root).
    #[error("{0} is not a Git repository (expected a bare repository or the root of a checkout)")]
    NotARepository(PathBuf),
    /// The path is inside a repository but is not its root (nor its git dir).
    #[error("{given} is not the root of a Git repository (the repository root is {root})")]
    NotRepositoryRoot { given: PathBuf, root: PathBuf },
    /// Any other failure while probing the repository.
    #[error("cannot open repository {path}: {message}")]
    Repo { path: PathBuf, message: String },
}
