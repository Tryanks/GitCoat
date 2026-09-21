//! Backend-independent data model of the Git access layer.
//!
//! Everything here is plain owned data (`Send + Sync`) so pages can hold it
//! across awaits, and none of it depends on how the objects were read.

use std::fmt;

use super::error::GitError;

/// Hash algorithm of the repository's objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}

impl ObjectFormat {
    /// Length of a full object id in hex digits.
    pub const fn hex_len(self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim() {
            "sha1" => Some(Self::Sha1),
            "sha256" => Some(Self::Sha256),
            _ => None,
        }
    }
}

/// A validated, full-length, lowercase hex object id.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Oid(String);

impl Oid {
    /// Validate `hex` as a full object id of `format`.
    pub fn parse(hex: &str, format: ObjectFormat) -> Result<Self, GitError> {
        Self::parse_len(hex, format.hex_len())
    }

    /// Validate `hex` as a full object id of exactly `len` hex digits.
    pub fn parse_len(hex: &str, len: usize) -> Result<Self, GitError> {
        if hex.len() == len && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Self(hex.to_ascii_lowercase()))
        } else {
            Err(GitError::invalid(format!(
                "`{hex}` is not a full object id"
            )))
        }
    }

    /// Wrap a hex string the backend already validated.
    pub(crate) fn from_validated(hex: String) -> Self {
        Self(hex)
    }

    /// Whether `hex` has the shape of a full object id of `format`.
    pub fn is_full_hex(hex: &str, format: ObjectFormat) -> bool {
        hex.len() == format.hex_len() && hex.bytes().all(|b| b.is_ascii_hexdigit())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The abbreviated form shown in the UI (first 7 digits).
    pub fn short(&self) -> &str {
        &self.0[..7.min(self.0.len())]
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({})", self.0)
    }
}

impl AsRef<str> for Oid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A normalized slash-separated path inside the repository tree.
///
/// The empty path is the tree root. Paths are byte strings because Git file
/// names need not be valid UTF-8; [`RepoPath::display`] (and `Display`) give a
/// lossy rendering that must only ever be *shown*, never used to look a file
/// up again. Use [`RepoPath::as_bytes`] for that.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoPath(Vec<u8>);

impl RepoPath {
    /// The tree root.
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// Parse a user-supplied path: no leading or trailing slash, no empty,
    /// `.` or `..` segments, no NUL bytes. An empty string is the root.
    pub fn parse(path: &str) -> Result<Self, GitError> {
        Self::from_bytes(path.as_bytes().to_vec())
    }

    /// Validate a path given as raw bytes (see [`RepoPath::parse`]).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, GitError> {
        if bytes.is_empty() {
            return Ok(Self::root());
        }
        if bytes.contains(&0) {
            return Err(GitError::invalid("path contains a NUL byte"));
        }
        for segment in bytes.split(|&b| b == b'/') {
            if segment.is_empty() {
                return Err(GitError::invalid("path must not contain empty segments"));
            }
            if segment == b"." || segment == b".." {
                return Err(GitError::invalid(
                    "path must not contain `.` or `..` segments",
                ));
            }
        }
        Ok(Self(bytes))
    }

    /// Build a path from already-validated segments (e.g. names from `ls-tree`).
    fn from_segments<'a>(segments: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut bytes = Vec::new();
        for segment in segments {
            if !bytes.is_empty() {
                bytes.push(b'/');
            }
            bytes.extend_from_slice(segment);
        }
        Self(bytes)
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The path as `&str` when it is valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    pub fn is_valid_utf8(&self) -> bool {
        self.as_str().is_some()
    }

    /// Lossy rendering for display purposes only.
    pub fn display(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }

    /// The path segments (none for the root).
    pub fn components(&self) -> impl Iterator<Item = &[u8]> {
        self.0
            .split(|&b| b == b'/')
            .filter(|segment| !segment.is_empty())
    }

    /// The parent directory; `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.iter().rposition(|&b| b == b'/') {
            Some(index) => Self(self.0[..index].to_vec()),
            None => Self::root(),
        })
    }

    /// The last segment; `None` for the root.
    pub fn file_name(&self) -> Option<&[u8]> {
        self.components().last()
    }

    /// Append one segment (a single file name, must not contain `/`).
    pub fn join(&self, name: &[u8]) -> Result<Self, GitError> {
        if name.is_empty()
            || name.contains(&b'/')
            || name.contains(&0)
            || name == b"."
            || name == b".."
        {
            return Err(GitError::invalid("invalid path segment"));
        }
        let mut segments: Vec<&[u8]> = self.components().collect();
        segments.push(name);
        Ok(Self::from_segments(segments))
    }

    /// Every ancestor from the root down to `self`, paired with its last
    /// segment: useful for breadcrumbs.
    pub fn ancestors(&self) -> Vec<(Self, Vec<u8>)> {
        let mut out = Vec::new();
        let mut current = Vec::new();
        for segment in self.components() {
            current.push(segment);
            out.push((
                Self::from_segments(current.iter().copied()),
                segment.to_vec(),
            ));
        }
        out
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

impl fmt::Debug for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepoPath({:?})", String::from_utf8_lossy(&self.0))
    }
}

/// What kind of ref a name denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Branch,
    Tag,
    /// A bare commit id given instead of a ref name.
    Commit,
}

/// A ref name (or raw commit id) resolved to the commit it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRef {
    /// The full ref name (`refs/heads/main`) or the full oid.
    pub name: String,
    /// The short name shown in the UI (`main`, `v1.0`, or the abbreviated oid).
    pub short: String,
    pub kind: RefKind,
    /// The commit the ref peels to.
    pub oid: Oid,
}

/// One branch or tag in the ref picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEntry {
    /// Full ref name.
    pub name: String,
    /// Short ref name.
    pub short: String,
    pub kind: RefKind,
    /// The commit the ref peels to.
    pub oid: Oid,
}

impl RefEntry {
    pub fn resolved(&self) -> ResolvedRef {
        ResolvedRef {
            name: self.name.clone(),
            short: self.short.clone(),
            kind: self.kind,
            oid: self.oid.clone(),
        }
    }
}

/// Branches and tags of the repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefList {
    /// Alphabetical, with the branch `HEAD` points at first.
    pub branches: Vec<RefEntry>,
    /// Version-sorted, newest first.
    pub tags: Vec<RefEntry>,
}

/// The kind of a tree entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Executable,
    Symlink,
    Submodule,
}

impl EntryKind {
    /// Classify an `ls-tree` mode string.
    pub fn from_mode(mode: &str) -> Self {
        match mode {
            "040000" | "40000" => Self::Dir,
            "100755" => Self::Executable,
            "120000" => Self::Symlink,
            "160000" => Self::Submodule,
            _ => Self::File,
        }
    }

    pub fn is_dir(self) -> bool {
        matches!(self, Self::Dir)
    }
}

/// One entry of a tree listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// The raw file name.
    pub name: Vec<u8>,
    /// Lossy rendering of `name`, for display only.
    pub name_display: String,
    pub kind: EntryKind,
    pub mode: String,
    pub oid: Oid,
    /// Size in bytes for blobs; `None` for trees and submodules.
    pub size: Option<u64>,
}

impl TreeEntry {
    pub fn is_valid_utf8(&self) -> bool {
        std::str::from_utf8(&self.name).is_ok()
    }
}

/// A tree listing, directories first, capped at `TREE_MAX_ENTRIES`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeListing {
    pub entries: Vec<TreeEntry>,
    /// `true` when entries were dropped to respect the cap.
    pub truncated: bool,
}

/// A README found in a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readme {
    pub entry: TreeEntry,
    /// The file content, cut at `README_MAX_BYTES`.
    pub data: Vec<u8>,
    pub truncated: bool,
}

/// Object type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Blob,
    Tree,
    Commit,
    Tag,
}

/// Type and size of an object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInfo {
    pub oid: Oid,
    pub size: u64,
    pub kind: ObjectKind,
}

/// A point in time as recorded by Git: Unix seconds plus the author's UTC
/// offset in minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub unix: i64,
    pub tz_offset_minutes: i32,
}

/// A commit as shown in a history list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    pub oid: Oid,
    /// Abbreviated oid.
    pub short: String,
    /// First line of the message.
    pub title: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: Timestamp,
    pub committer_time: Timestamp,
    pub parents: Vec<Oid>,
}

/// A commit with its full message and committer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetail {
    pub summary: CommitSummary,
    /// The message without its first line (and the separating blank line).
    pub body: String,
    pub committer_name: String,
    pub committer_email: String,
}

impl std::ops::Deref for CommitDetail {
    type Target = CommitSummary;

    fn deref(&self) -> &Self::Target {
        &self.summary
    }
}

/// The change a diff file describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffStatus {
    Added,
    Deleted,
    Modified,
    Renamed {
        from: RepoPath,
    },
    Copied {
        from: RepoPath,
    },
    /// Only the mode changed (including type changes such as file → symlink).
    ModeChanged,
}

/// The kind of a diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u64>,
    pub new_no: Option<u64>,
    /// Line content without the leading marker (lossy UTF-8).
    pub text: String,
}

/// One `@@` hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The full `@@ ... @@ context` header line.
    pub header: String,
    pub old_start: u64,
    pub old_count: u64,
    pub new_start: u64,
    pub new_count: u64,
    pub lines: Vec<DiffLine>,
}

/// One file of a commit diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub status: DiffStatus,
    /// Path before the change; `None` for added files.
    pub old_path: Option<RepoPath>,
    /// Path after the change; `None` for deleted files.
    pub new_path: Option<RepoPath>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    /// Added lines; `None` when unknown (binary).
    pub additions: Option<u64>,
    /// Deleted lines; `None` when unknown (binary).
    pub deletions: Option<u64>,
    pub hunks: Vec<Hunk>,
    pub is_binary: bool,
    /// `true` when the patch for this file was cut off by a size limit.
    pub truncated: bool,
}

impl DiffFile {
    /// The path to show for the file (new path, or old path for deletions).
    pub fn path(&self) -> &RepoPath {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .expect("a diff file has at least one path")
    }
}

/// The diff of a commit against its first parent (or the empty tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub files: Vec<DiffFile>,
    /// `true` when files or hunks were dropped to respect a limit.
    pub truncated: bool,
    pub additions: u64,
    pub deletions: u64,
    /// The first parent the commit was compared against; `None` for a root
    /// commit (compared against the empty tree).
    pub compared_against: Option<Oid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_validation() {
        let sha1 = "a".repeat(40);
        assert!(Oid::parse(&sha1, ObjectFormat::Sha1).is_ok());
        assert!(Oid::parse(&sha1, ObjectFormat::Sha256).is_err());
        assert!(Oid::parse(&"g".repeat(40), ObjectFormat::Sha1).is_err());
        assert!(Oid::parse("abc", ObjectFormat::Sha1).is_err());
        assert_eq!(
            Oid::parse(&"A".repeat(64), ObjectFormat::Sha256)
                .unwrap()
                .short(),
            "aaaaaaa"
        );
    }

    #[test]
    fn repo_path_validation() {
        assert!(RepoPath::parse("").unwrap().is_root());
        assert_eq!(
            RepoPath::parse("src/main.rs").unwrap().display(),
            "src/main.rs"
        );
        for bad in [
            "/abs",
            "trailing/",
            "a//b",
            ".",
            "..",
            "a/../b",
            "./a",
            "nul\0",
        ] {
            assert!(RepoPath::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn repo_path_navigation() {
        let path = RepoPath::parse("a/b/c.txt").unwrap();
        assert_eq!(path.file_name(), Some(&b"c.txt"[..]));
        assert_eq!(path.parent().unwrap().display(), "a/b");
        assert_eq!(
            RepoPath::parse("a").unwrap().parent().unwrap(),
            RepoPath::root()
        );
        assert!(RepoPath::root().parent().is_none());
        assert_eq!(RepoPath::root().join(b"x").unwrap().display(), "x");
        assert_eq!(
            path.parent().unwrap().join(b"d").unwrap().display(),
            "a/b/d"
        );
        assert!(path.join(b"x/y").is_err());
        assert!(path.join(b"..").is_err());
        let crumbs = path.ancestors();
        assert_eq!(crumbs.len(), 3);
        assert_eq!(crumbs[1].0.display(), "a/b");
        assert_eq!(crumbs[2].1, b"c.txt");
    }

    #[test]
    fn repo_path_non_utf8_is_lossless() {
        let raw = vec![b'd', b'i', b'r', b'/', 0xff, b'x'];
        let path = RepoPath::from_bytes(raw.clone()).unwrap();
        assert!(!path.is_valid_utf8());
        assert_eq!(path.as_bytes(), raw.as_slice());
        assert_eq!(path.display(), "dir/\u{fffd}x");
    }

    #[test]
    fn entry_kind_from_mode() {
        assert_eq!(EntryKind::from_mode("040000"), EntryKind::Dir);
        assert_eq!(EntryKind::from_mode("100644"), EntryKind::File);
        assert_eq!(EntryKind::from_mode("100755"), EntryKind::Executable);
        assert_eq!(EntryKind::from_mode("120000"), EntryKind::Symlink);
        assert_eq!(EntryKind::from_mode("160000"), EntryKind::Submodule);
    }
}
