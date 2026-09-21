//! Resource limits shared by the Git backend and the pages.

use std::time::Duration;

/// Maximum number of concurrently running repository operations.
pub const GIT_CONCURRENCY: usize = 4;
/// Wall-clock limit for a single repository operation.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest object read into memory by any operation (the `/raw` gate).
pub const RAW_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Largest blob rendered as text.
pub const TEXT_PREVIEW_MAX_BYTES: usize = 1024 * 1024;
/// Maximum number of lines rendered for a text blob.
pub const TEXT_PREVIEW_MAX_LINES: usize = 10_000;
/// Largest blob rendered inline as an image.
pub const IMAGE_PREVIEW_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Largest text run through the syntax highlighter; bigger text is only escaped.
pub const HIGHLIGHT_MAX_BYTES: usize = 512 * 1024;

/// Cap on the blob bytes read while building a commit diff (all files together).
pub const DIFF_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Maximum number of files shown in a commit diff.
pub const DIFF_MAX_FILES: usize = 300;
/// Maximum number of entries listed for one tree.
pub const TREE_MAX_ENTRIES: usize = 2000;
/// Commits per page on the history view.
pub const COMMITS_PER_PAGE: usize = 50;
/// Deepest history offset a page may ask for (`skip` in [`crate::git::Repo::log`]).
pub const COMMITS_MAX_SKIP: usize = 50_000;
/// Largest README rendered on the tree page.
pub const README_MAX_BYTES: usize = 1024 * 1024;
/// Maximum number of branches or tags listed in the ref picker.
pub const REFS_MAX_ENTRIES: usize = 10_000;
