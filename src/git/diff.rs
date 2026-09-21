//! Commit diffs: tree comparison against the first parent plus per-file
//! unified hunks with line numbers.

use gix::{
    bstr::{BStr, ByteSlice},
    diff::blob::{
        Algorithm, InternedInput, UnifiedDiff, diff_with_slider_heuristics,
        unified_diff::{ConsumeHunk, ContextSize, DiffLineKind, HunkHeader},
    },
    objs::tree::EntryMode,
};

use super::{
    Diff, DiffFile, DiffLine, DiffStatus, GitError, Hunk, LineKind, Oid, Repo, RepoPath,
    convert::{looks_binary, mode_string, oid_from_gix, oid_to_gix, repo_error},
};
use crate::limits::{DIFF_MAX_BYTES, DIFF_MAX_FILES};

type Change = gix::diff::tree_with_rewrites::Change;

impl Repo {
    /// The changes `commit` introduced relative to its first parent (or the
    /// empty tree for a root commit).
    pub async fn diff(&self, commit: &Oid) -> Result<Diff, GitError> {
        let commit = commit.clone();
        self.run(move |repo| diff(repo, &commit)).await
    }
}

fn diff(repo: &gix::Repository, oid: &Oid) -> Result<Diff, GitError> {
    let id = oid_to_gix(oid)?;
    let Some(object) = repo
        .try_find_object(id)
        .map_err(|e| repo_error("looking up commit", e))?
    else {
        return Err(GitError::not_found(format!("commit {oid} does not exist")));
    };
    if object.kind != gix::objs::Kind::Commit {
        return Err(GitError::not_found(format!("{oid} is not a commit")));
    }
    let commit = object.into_commit();
    let new_tree = commit
        .tree()
        .map_err(|e| repo_error("reading commit tree", e))?;
    let first_parent = commit.parent_ids().next().map(|id| id.detach());
    let old_tree = match &first_parent {
        Some(parent) => Some(
            repo.find_commit(*parent)
                .map_err(|e| repo_error("reading parent commit", e))?
                .tree()
                .map_err(|e| repo_error("reading parent tree", e))?,
        ),
        None => None,
    };

    let options = gix::diff::Options::default().with_rewrites(Some(gix::diff::Rewrites::default()));
    let changes = repo
        .diff_tree_to_tree(old_tree.as_ref(), &new_tree, options)
        .map_err(|e| repo_error("comparing trees", e))?;

    let mut budget = Budget {
        remaining: DIFF_MAX_BYTES,
        exhausted: false,
    };
    let mut files = Vec::new();
    let mut truncated = false;
    for change in changes {
        if is_tree_change(&change) {
            continue;
        }
        if files.len() >= DIFF_MAX_FILES {
            truncated = true;
            break;
        }
        files.push(diff_file(repo, change, &mut budget)?);
    }
    truncated |= budget.exhausted;

    let additions = files.iter().filter_map(|f| f.additions).sum();
    let deletions = files.iter().filter_map(|f| f.deletions).sum();
    Ok(Diff {
        files,
        truncated,
        additions,
        deletions,
        compared_against: first_parent.map(|id| oid_from_gix(&id)),
    })
}

fn is_tree_change(change: &Change) -> bool {
    match change {
        Change::Addition { entry_mode, .. } | Change::Deletion { entry_mode, .. } => {
            entry_mode.is_tree()
        }
        Change::Modification {
            entry_mode,
            previous_entry_mode,
            ..
        } => entry_mode.is_tree() && previous_entry_mode.is_tree(),
        Change::Rewrite {
            entry_mode,
            source_entry_mode,
            ..
        } => entry_mode.is_tree() && source_entry_mode.is_tree(),
    }
}

/// Bytes still allowed to be read for hunks in this diff.
struct Budget {
    remaining: usize,
    exhausted: bool,
}

/// One side of a file comparison.
enum Side {
    /// No content on this side (added/deleted file, or a submodule).
    Absent,
    Content(Vec<u8>),
    /// The blob is too large for the remaining budget.
    TooLarge,
}

fn read_side(
    repo: &gix::Repository,
    mode: EntryMode,
    id: &gix::oid,
    budget: &mut Budget,
) -> Result<Side, GitError> {
    if !mode.is_blob_or_symlink() {
        return Ok(Side::Absent);
    }
    let header = repo
        .find_header(id)
        .map_err(|e| repo_error("reading object header", e))?;
    let size = usize::try_from(header.size()).unwrap_or(usize::MAX);
    if size > budget.remaining {
        budget.exhausted = true;
        return Ok(Side::TooLarge);
    }
    budget.remaining -= size;
    let mut blob = repo
        .find_blob(id)
        .map_err(|e| repo_error("reading blob", e))?;
    Ok(Side::Content(blob.take_data()))
}

fn path_of(location: &BStr) -> RepoPath {
    RepoPath::from_bytes(location.to_vec()).unwrap_or_else(|_| RepoPath::root())
}

fn diff_file(
    repo: &gix::Repository,
    change: Change,
    budget: &mut Budget,
) -> Result<DiffFile, GitError> {
    let (status, old, new) = match change {
        Change::Addition {
            location,
            entry_mode,
            id,
            ..
        } => (
            DiffStatus::Added,
            None,
            Some((path_of(location.as_bstr()), entry_mode, id)),
        ),
        Change::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => (
            DiffStatus::Deleted,
            Some((path_of(location.as_bstr()), entry_mode, id)),
            None,
        ),
        Change::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => {
            let path = path_of(location.as_bstr());
            let status = if previous_id == id {
                DiffStatus::ModeChanged
            } else {
                DiffStatus::Modified
            };
            (
                status,
                Some((path.clone(), previous_entry_mode, previous_id)),
                Some((path, entry_mode, id)),
            )
        }
        Change::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            copy,
            ..
        } => {
            let from = path_of(source_location.as_bstr());
            let status = if copy {
                DiffStatus::Copied { from: from.clone() }
            } else {
                DiffStatus::Renamed { from: from.clone() }
            };
            (
                status,
                Some((from, source_entry_mode, source_id)),
                Some((path_of(location.as_bstr()), entry_mode, id)),
            )
        }
    };

    let mut file = DiffFile {
        status,
        old_path: old.as_ref().map(|(path, ..)| path.clone()),
        new_path: new.as_ref().map(|(path, ..)| path.clone()),
        old_mode: old.as_ref().map(|(_, mode, _)| mode_string(*mode)),
        new_mode: new.as_ref().map(|(_, mode, _)| mode_string(*mode)),
        additions: Some(0),
        deletions: Some(0),
        hunks: Vec::new(),
        is_binary: false,
        truncated: false,
    };

    let same_content = match (&old, &new) {
        (Some((_, _, a)), Some((_, _, b))) => a == b,
        _ => false,
    };
    if same_content {
        return Ok(file);
    }

    let old_side = match &old {
        Some((_, mode, id)) => read_side(repo, *mode, id, budget)?,
        None => Side::Absent,
    };
    let new_side = match &new {
        Some((_, mode, id)) => read_side(repo, *mode, id, budget)?,
        None => Side::Absent,
    };

    let (old_data, new_data) = match (old_side, new_side) {
        (Side::TooLarge, _) | (_, Side::TooLarge) => {
            file.truncated = true;
            file.additions = None;
            file.deletions = None;
            return Ok(file);
        }
        (Side::Absent, Side::Absent) => {
            // Submodule pointer change: nothing to compare.
            return Ok(file);
        }
        (old, new) => (side_bytes(old), side_bytes(new)),
    };

    if looks_binary(&old_data) || looks_binary(&new_data) {
        file.is_binary = true;
        file.additions = None;
        file.deletions = None;
        return Ok(file);
    }

    let hunks = unified_hunks(&old_data, &new_data)?;
    file.additions = Some(
        hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Add)
            .count() as u64,
    );
    file.deletions = Some(
        hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == LineKind::Del)
            .count() as u64,
    );
    file.hunks = hunks;
    Ok(file)
}

fn side_bytes(side: Side) -> Vec<u8> {
    match side {
        Side::Content(data) => data,
        Side::Absent | Side::TooLarge => Vec::new(),
    }
}

/// Unified hunks (3 lines of context) between two text blobs, with old and
/// new line numbers attached to every line.
pub(super) fn unified_hunks(old: &[u8], new: &[u8]) -> Result<Vec<Hunk>, GitError> {
    let input = InternedInput::new(old, new);
    let diff = diff_with_slider_heuristics(Algorithm::Histogram, &input);
    UnifiedDiff::new(
        &diff,
        &input,
        HunkCollector::default(),
        ContextSize::symmetrical(3),
    )
    .consume()
    .map_err(|e| repo_error("building unified diff", e))
}

/// Collects hunks from gitoxide's unified diff writer.
#[derive(Default)]
struct HunkCollector {
    hunks: Vec<Hunk>,
}

impl ConsumeHunk for HunkCollector {
    type Out = Vec<Hunk>;

    fn consume_hunk(
        &mut self,
        header: HunkHeader,
        lines: &[(DiffLineKind, &[u8])],
    ) -> std::io::Result<()> {
        let mut old_no = u64::from(header.before_hunk_start);
        let mut new_no = u64::from(header.after_hunk_start);
        let mut out = Vec::with_capacity(lines.len());
        for &(kind, content) in lines {
            let text = strip_newline(content).to_str_lossy().into_owned();
            let line = match kind {
                DiffLineKind::Context => {
                    let line = DiffLine {
                        kind: LineKind::Context,
                        old_no: Some(old_no),
                        new_no: Some(new_no),
                        text,
                    };
                    old_no += 1;
                    new_no += 1;
                    line
                }
                DiffLineKind::Add => {
                    let line = DiffLine {
                        kind: LineKind::Add,
                        old_no: None,
                        new_no: Some(new_no),
                        text,
                    };
                    new_no += 1;
                    line
                }
                DiffLineKind::Remove => {
                    let line = DiffLine {
                        kind: LineKind::Del,
                        old_no: Some(old_no),
                        new_no: None,
                        text,
                    };
                    old_no += 1;
                    line
                }
            };
            out.push(line);
        }
        // Git prints an empty range as `-0,0` / `+0,0` (start one before the
        // first line); gitoxide reports the line the range would start at.
        let (old_start, old_count) = git_range(header.before_hunk_start, header.before_hunk_len);
        let (new_start, new_count) = git_range(header.after_hunk_start, header.after_hunk_len);
        self.hunks.push(Hunk {
            header: format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@"),
            old_start,
            old_count,
            new_start,
            new_count,
            lines: out,
        });
        Ok(())
    }

    fn finish(self) -> Self::Out {
        self.hunks
    }
}

fn git_range(start: u32, len: u32) -> (u64, u64) {
    let start = if len == 0 {
        start.saturating_sub(1)
    } else {
        start
    };
    (u64::from(start), u64::from(len))
}

fn strip_newline(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(hunk: &Hunk) -> Vec<(LineKind, Option<u64>, Option<u64>, &str)> {
        hunk.lines
            .iter()
            .map(|l| (l.kind, l.old_no, l.new_no, l.text.as_str()))
            .collect()
    }

    #[test]
    fn modification_has_numbered_lines() {
        let hunks = unified_hunks(b"a\nb\nc\nd\n", b"a\nB\nc\nd\n").unwrap();
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!(hunk.header, "@@ -1,4 +1,4 @@");
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_count,
                hunk.new_start,
                hunk.new_count
            ),
            (1, 4, 1, 4)
        );
        assert_eq!(
            kinds(hunk),
            vec![
                (LineKind::Context, Some(1), Some(1), "a"),
                (LineKind::Del, Some(2), None, "b"),
                (LineKind::Add, None, Some(2), "B"),
                (LineKind::Context, Some(3), Some(3), "c"),
                (LineKind::Context, Some(4), Some(4), "d"),
            ]
        );
    }

    #[test]
    fn added_file_is_one_hunk_of_additions() {
        let hunks = unified_hunks(b"", b"one\ntwo\r\n").unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].header, "@@ -0,0 +1,2 @@");
        assert_eq!(
            kinds(&hunks[0]),
            vec![
                (LineKind::Add, None, Some(1), "one"),
                (LineKind::Add, None, Some(2), "two")
            ]
        );
    }

    #[test]
    fn distant_changes_make_separate_hunks() {
        let old: String = (1..=30).map(|i| format!("line {i}\n")).collect();
        let new = old
            .replace("line 2\n", "LINE 2\n")
            .replace("line 28\n", "LINE 28\n");
        let hunks = unified_hunks(old.as_bytes(), new.as_bytes()).unwrap();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header, "@@ -1,5 +1,5 @@");
        assert_eq!(hunks[1].header, "@@ -25,6 +25,6 @@");
        assert_eq!(hunks[1].lines[0].old_no, Some(25));
        assert_eq!(
            hunks[1]
                .lines
                .iter()
                .find(|l| l.kind == LineKind::Add)
                .unwrap()
                .new_no,
            Some(28)
        );
    }

    #[test]
    fn identical_input_has_no_hunks() {
        assert!(unified_hunks(b"same\n", b"same\n").unwrap().is_empty());
    }

    #[test]
    fn binary_detection() {
        assert!(looks_binary(b"abc\0def"));
        assert!(!looks_binary("plain text ünïcödé\n".as_bytes()));
        let mut late_nul = vec![b'a'; 9000];
        late_nul.push(0);
        assert!(!looks_binary(&late_nul));
    }
}
