//! Tree listings, path lookup and blob reads.

use super::{
    BlobInfo, EntryKind, GitError, ObjectKind, Oid, Readme, Repo, RepoPath, TreeEntry, TreeListing,
    convert::{oid_from_gix, oid_to_gix, repo_error, tree_entry},
};
use crate::limits::{RAW_MAX_BYTES, README_MAX_BYTES, TREE_MAX_ENTRIES};

/// README candidates in priority order (matched case-insensitively).
const README_NAMES: [&str; 3] = ["README.md", "README.markdown", "README"];

impl Repo {
    /// List the directory `dir` of `commit`'s tree, directories first, capped
    /// at `TREE_MAX_ENTRIES`. `None` if `dir` does not exist or is not a
    /// directory on this commit.
    pub async fn ls_tree(
        &self,
        commit: &Oid,
        dir: &RepoPath,
    ) -> Result<Option<TreeListing>, GitError> {
        let commit = commit.clone();
        let dir = dir.clone();
        self.run(move |repo| ls_tree(repo, &commit, &dir)).await
    }

    /// The tree entry at `path` on `commit` (the root tree for the empty
    /// path). `None` if the path does not exist.
    pub async fn entry(
        &self,
        commit: &Oid,
        path: &RepoPath,
    ) -> Result<Option<TreeEntry>, GitError> {
        let commit = commit.clone();
        let path = path.clone();
        self.run(move |repo| entry(repo, &commit, &path)).await
    }

    /// Type and size of an object without reading its content.
    pub async fn blob_info(&self, oid: &Oid) -> Result<Option<BlobInfo>, GitError> {
        let oid = oid.clone();
        self.run(move |repo| blob_info(repo, &oid)).await
    }

    /// Read a blob, returning at most `max_bytes` and whether it was cut.
    ///
    /// Objects larger than `RAW_MAX_BYTES` are refused with
    /// [`GitError::Limit`] before anything is read. `None` if the object does
    /// not exist or is not a blob.
    pub async fn read_blob(
        &self,
        oid: &Oid,
        max_bytes: usize,
    ) -> Result<Option<(Vec<u8>, bool)>, GitError> {
        let oid = oid.clone();
        self.run(move |repo| read_blob(repo, &oid, max_bytes)).await
    }

    /// The README of `dir` on `commit`, if any (`README.md`, then
    /// `README.markdown`, then `README`, case-insensitive).
    pub async fn readme(&self, commit: &Oid, dir: &RepoPath) -> Result<Option<Readme>, GitError> {
        let commit = commit.clone();
        let dir = dir.clone();
        self.run(move |repo| readme(repo, &commit, &dir)).await
    }
}

/// The tree object for `dir` on `commit`, or `None` if `dir` is missing or
/// not a directory.
fn find_dir<'r>(
    repo: &'r gix::Repository,
    commit: &Oid,
    dir: &RepoPath,
) -> Result<Option<gix::Tree<'r>>, GitError> {
    let Some(mut root) = root_tree(repo, commit)? else {
        return Ok(None);
    };
    if dir.is_root() {
        return Ok(Some(root));
    }
    let entry = root
        .peel_to_entry(dir.components())
        .map_err(|e| repo_error("walking tree", e))?;
    let Some(entry) = entry else {
        return Ok(None);
    };
    if !entry.mode().is_tree() {
        return Ok(None);
    }
    let tree = repo
        .find_tree(entry.oid())
        .map_err(|e| repo_error("reading tree", e))?;
    Ok(Some(tree))
}

/// The root tree of `commit`; `None` when `commit` is not a commit.
fn root_tree<'r>(
    repo: &'r gix::Repository,
    commit: &Oid,
) -> Result<Option<gix::Tree<'r>>, GitError> {
    let id = oid_to_gix(commit)?;
    let Some(object) = repo
        .try_find_object(id)
        .map_err(|e| repo_error("looking up commit", e))?
    else {
        return Ok(None);
    };
    if object.kind != gix::objs::Kind::Commit {
        return Ok(None);
    }
    let tree = object
        .into_commit()
        .tree()
        .map_err(|e| repo_error("reading commit tree", e))?;
    Ok(Some(tree))
}

fn ls_tree(
    repo: &gix::Repository,
    commit: &Oid,
    dir: &RepoPath,
) -> Result<Option<TreeListing>, GitError> {
    let Some(tree) = find_dir(repo, commit, dir)? else {
        return Ok(None);
    };
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in tree.iter() {
        let entry = entry.map_err(|e| repo_error("decoding tree", e))?;
        if entries.len() >= TREE_MAX_ENTRIES {
            truncated = true;
            break;
        }
        let mode = entry.mode();
        let size = if mode.is_blob_or_symlink() {
            repo.try_find_header(entry.oid())
                .map_err(|e| repo_error("reading object header", e))?
                .map(|header| header.size())
        } else {
            None
        };
        entries.push(tree_entry(entry.filename(), mode, entry.oid(), size));
    }
    entries.sort_by(|a, b| {
        b.kind
            .is_dir()
            .cmp(&a.kind.is_dir())
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(Some(TreeListing { entries, truncated }))
}

fn entry(
    repo: &gix::Repository,
    commit: &Oid,
    path: &RepoPath,
) -> Result<Option<TreeEntry>, GitError> {
    let Some(mut root) = root_tree(repo, commit)? else {
        return Ok(None);
    };
    if path.is_root() {
        let mode = gix::objs::tree::EntryMode::from(gix::objs::tree::EntryKind::Tree);
        return Ok(Some(tree_entry(b"", mode, &root.id, None)));
    }
    let entry = root
        .peel_to_entry(path.components())
        .map_err(|e| repo_error("walking tree", e))?;
    let Some(entry) = entry else {
        return Ok(None);
    };
    let mode = entry.mode();
    let size = if mode.is_blob_or_symlink() {
        repo.try_find_header(entry.oid())
            .map_err(|e| repo_error("reading object header", e))?
            .map(|header| header.size())
    } else {
        None
    };
    Ok(Some(tree_entry(entry.filename(), mode, entry.oid(), size)))
}

fn blob_info(repo: &gix::Repository, oid: &Oid) -> Result<Option<BlobInfo>, GitError> {
    let id = oid_to_gix(oid)?;
    let Some(header) = repo
        .try_find_header(id)
        .map_err(|e| repo_error("reading object header", e))?
    else {
        return Ok(None);
    };
    let kind = match header.kind() {
        gix::objs::Kind::Blob => ObjectKind::Blob,
        gix::objs::Kind::Tree => ObjectKind::Tree,
        gix::objs::Kind::Commit => ObjectKind::Commit,
        gix::objs::Kind::Tag => ObjectKind::Tag,
    };
    Ok(Some(BlobInfo {
        oid: oid_from_gix(&id),
        size: header.size(),
        kind,
    }))
}

fn read_blob(
    repo: &gix::Repository,
    oid: &Oid,
    max_bytes: usize,
) -> Result<Option<(Vec<u8>, bool)>, GitError> {
    let Some(info) = blob_info(repo, oid)? else {
        return Ok(None);
    };
    if info.kind != ObjectKind::Blob {
        return Ok(None);
    }
    if info.size > RAW_MAX_BYTES as u64 {
        return Err(GitError::limit(format!(
            "blob {oid} is {} bytes, larger than the {RAW_MAX_BYTES} byte limit",
            info.size
        )));
    }
    let id = oid_to_gix(oid)?;
    let mut blob = repo
        .find_blob(id)
        .map_err(|e| repo_error("reading blob", e))?;
    let mut data = blob.take_data();
    let truncated = data.len() > max_bytes;
    if truncated {
        data.truncate(max_bytes);
    }
    Ok(Some((data, truncated)))
}

fn readme(
    repo: &gix::Repository,
    commit: &Oid,
    dir: &RepoPath,
) -> Result<Option<Readme>, GitError> {
    let Some(listing) = ls_tree(repo, commit, dir)? else {
        return Ok(None);
    };
    let entry = README_NAMES.iter().find_map(|wanted| {
        listing.entries.iter().find(|entry| {
            matches!(entry.kind, EntryKind::File | EntryKind::Executable)
                && entry.name.eq_ignore_ascii_case(wanted.as_bytes())
        })
    });
    let Some(entry) = entry else {
        return Ok(None);
    };
    let Some((data, truncated)) = read_blob(repo, &entry.oid, README_MAX_BYTES)? else {
        return Ok(None);
    };
    Ok(Some(Readme {
        entry: entry.clone(),
        data,
        truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_names_are_ordered_by_priority() {
        assert_eq!(README_NAMES[0], "README.md");
        assert!(b"readme.MD".eq_ignore_ascii_case(README_NAMES[0].as_bytes()));
    }
}
