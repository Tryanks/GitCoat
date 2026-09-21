//! The `Repo` handle: opening, validation and the bounded execution of
//! blocking gitoxide work.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::Semaphore;

use super::{GitError, ObjectFormat, OpenError};
use crate::limits::{GIT_CONCURRENCY, GIT_TIMEOUT};

/// Object cache handed to each thread-local repository (speeds up walks/diffs).
const OBJECT_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// A read-only handle to one repository, safe to share across requests.
///
/// Every query runs on the blocking thread pool through [`Repo::run`], which
/// bounds concurrency with a semaphore and applies the global timeout.
pub struct Repo {
    shared: Arc<gix::ThreadSafeRepository>,
    path: PathBuf,
    git_dir: PathBuf,
    is_bare: bool,
    format: ObjectFormat,
    semaphore: Arc<Semaphore>,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo")
            .field("path", &self.path)
            .field("git_dir", &self.git_dir)
            .field("is_bare", &self.is_bare)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl Repo {
    /// Open the repository at `path`, which must be a bare repository, the root
    /// of a checkout, or a checkout's `.git` directory.
    ///
    /// The path is trusted because the operator chose it explicitly, so no
    /// `safe.directory` configuration is needed. Repository-local
    /// configuration is read; the process environment (`GIT_DIR` & co.) is
    /// ignored.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        let given = path.as_ref();
        let canonical = given
            .canonicalize()
            .map_err(|_| OpenError::NotFound(given.to_path_buf()))?;
        if !canonical.is_dir() {
            return Err(OpenError::NotFound(given.to_path_buf()));
        }

        let options = gix::open::Options::isolated().with(gix::sec::Trust::Full);
        let shared = match gix::ThreadSafeRepository::open_opts(&canonical, options) {
            Ok(repo) => repo,
            Err(gix::open::Error::NotARepository { .. }) => {
                return Err(OpenError::NotARepository(given.to_path_buf()));
            }
            Err(error) => {
                return Err(OpenError::Repo {
                    path: given.to_path_buf(),
                    message: error.to_string(),
                });
            }
        };

        let repo = shared.to_thread_local();
        let git_dir = repo
            .git_dir()
            .canonicalize()
            .unwrap_or_else(|_| repo.git_dir().to_path_buf());
        let workdir = repo
            .workdir()
            .map(|dir| dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()));
        let is_bare = repo.is_bare();
        let root = workdir.clone().unwrap_or_else(|| git_dir.clone());
        let is_root = git_dir == canonical || workdir.as_deref() == Some(canonical.as_path());
        if !is_root {
            return Err(OpenError::NotRepositoryRoot {
                given: given.to_path_buf(),
                root,
            });
        }

        let format = match repo.object_hash() {
            gix::hash::Kind::Sha1 => ObjectFormat::Sha1,
            gix::hash::Kind::Sha256 => ObjectFormat::Sha256,
            other => {
                return Err(OpenError::Repo {
                    path: given.to_path_buf(),
                    message: format!("unsupported object format {other:?}"),
                });
            }
        };

        Ok(Self {
            shared: Arc::new(shared),
            path: canonical,
            git_dir,
            is_bare,
            format,
            semaphore: Arc::new(Semaphore::new(GIT_CONCURRENCY)),
        })
    }

    /// Whether the repository has no working tree.
    pub fn is_bare(&self) -> bool {
        self.is_bare
    }

    /// The canonical path the repository was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The canonical `.git` directory (or the bare repository itself).
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// The hash algorithm of the repository's objects.
    pub fn object_format(&self) -> ObjectFormat {
        self.format
    }

    /// Run `work` on the blocking pool with a fresh thread-local repository,
    /// bounded by the concurrency semaphore and the global timeout.
    ///
    /// A timed-out task keeps running until its own iteration caps stop it,
    /// holding its permit meanwhile, which is what keeps the server bounded.
    pub(super) async fn run<T, F>(&self, work: F) -> Result<T, GitError>
    where
        T: Send + 'static,
        F: FnOnce(&gix::Repository) -> Result<T, GitError> + Send + 'static,
    {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| GitError::repo("repository handle closed"))?;
        let shared = Arc::clone(&self.shared);
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut repo = shared.to_thread_local();
            repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
            work(&repo)
        });
        match tokio::time::timeout(GIT_TIMEOUT, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) => Err(GitError::repo(format!("repository task failed: {join}"))),
            Err(_) => Err(GitError::Timeout),
        }
    }
}
