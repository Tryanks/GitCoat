//! Branches, tags, HEAD and user-supplied ref resolution.

use std::cmp::Ordering;

use gix::bstr::{BStr, ByteSlice};

use super::{
    GitError, ObjectFormat, Oid, RefEntry, RefKind, RefList, Repo, ResolvedRef,
    convert::{oid_from_gix, repo_error},
};
use crate::limits::REFS_MAX_ENTRIES;

const HEADS: &str = "refs/heads/";
const TAGS: &str = "refs/tags/";

impl Repo {
    /// All local branches and tags that point (after peeling) at commits.
    pub async fn list_refs(&self) -> Result<RefList, GitError> {
        self.run(list_refs).await
    }

    /// The ref the home page shows: `HEAD` when it points at a commit, else
    /// `main`, `master`, the first branch, the first tag. `None` for an empty
    /// repository.
    pub async fn default_ref(&self) -> Result<Option<ResolvedRef>, GitError> {
        self.run(default_ref).await
    }

    /// Resolve a user-supplied ref: a full branch or tag name, or a full hex
    /// object id (commit, or annotated tag peeled to its commit). Anything
    /// else is `None`; the input is never interpreted as a revspec.
    pub async fn resolve(&self, refspec: &str) -> Result<Option<ResolvedRef>, GitError> {
        let refspec = refspec.to_owned();
        let format = self.object_format();
        self.run(move |repo| resolve(repo, &refspec, format)).await
    }
}

fn list_refs(repo: &gix::Repository) -> Result<RefList, GitError> {
    let head_branch = head_branch_name(repo);
    let platform = repo
        .references()
        .map_err(|e| repo_error("reading refs", e))?;

    let mut branches = collect(repo, platform.local_branches(), RefKind::Branch)?;
    branches.sort_by(|a, b| {
        let a_head = Some(a.name.as_str()) == head_branch.as_deref();
        let b_head = Some(b.name.as_str()) == head_branch.as_deref();
        b_head
            .cmp(&a_head)
            .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
    });

    let mut tags = collect(repo, platform.tags(), RefKind::Tag)?;
    tags.sort_by(|a, b| version_cmp(&b.short, &a.short).then_with(|| b.name.cmp(&a.name)));

    Ok(RefList { branches, tags })
}

type RefIterResult<'a, 'r> =
    Result<gix::reference::iter::Iter<'a, 'r>, gix::reference::iter::init::Error>;

fn collect(
    repo: &gix::Repository,
    iter: RefIterResult<'_, '_>,
    kind: RefKind,
) -> Result<Vec<RefEntry>, GitError> {
    let iter = iter
        .map_err(|e| repo_error("listing refs", e))?
        .peeled()
        .map_err(|e| repo_error("reading packed-refs", e))?;
    let mut out = Vec::new();
    for reference in iter {
        if out.len() >= REFS_MAX_ENTRIES {
            break;
        }
        // Broken refs (dangling symrefs, unparsable files) are skipped rather
        // than failing the whole listing.
        let Ok(reference) = reference else { continue };
        let Some(id) = reference.try_id() else {
            continue;
        };
        if !is_commit(repo, &id) {
            continue;
        }
        let name = reference.name();
        out.push(RefEntry {
            name: name.as_bstr().to_str_lossy().into_owned(),
            short: name.shorten().to_str_lossy().into_owned(),
            kind,
            oid: oid_from_gix(&id),
        });
    }
    Ok(out)
}

fn is_commit(repo: &gix::Repository, id: &gix::oid) -> bool {
    matches!(repo.try_find_header(id), Ok(Some(header)) if header.kind() == gix::objs::Kind::Commit)
}

/// The branch `HEAD` points at (even if unborn), as a full name.
fn head_branch_name(repo: &gix::Repository) -> Option<String> {
    let head = repo.head().ok()?;
    head.referent_name()
        .map(|name| name.as_bstr().to_str_lossy().into_owned())
}

fn default_ref(repo: &gix::Repository) -> Result<Option<ResolvedRef>, GitError> {
    if let Ok(mut head) = repo.head()
        && let Ok(Some(id)) = head.try_peel_to_id()
        && is_commit(repo, &id)
    {
        let oid = oid_from_gix(&id);
        return Ok(Some(match head.referent_name() {
            Some(name) => ResolvedRef {
                name: name.as_bstr().to_str_lossy().into_owned(),
                short: name.shorten().to_str_lossy().into_owned(),
                kind: RefKind::Branch,
                oid,
            },
            None => commit_ref(oid),
        }));
    }

    let refs = list_refs(repo)?;
    let preferred = ["refs/heads/main", "refs/heads/master"];
    let pick = preferred
        .iter()
        .find_map(|wanted| refs.branches.iter().find(|b| b.name == *wanted))
        .or_else(|| refs.branches.first())
        .or_else(|| refs.tags.first());
    Ok(pick.map(RefEntry::resolved))
}

fn commit_ref(oid: Oid) -> ResolvedRef {
    ResolvedRef {
        name: oid.as_str().to_owned(),
        short: oid.short().to_owned(),
        kind: RefKind::Commit,
        oid,
    }
}

fn resolve(
    repo: &gix::Repository,
    refspec: &str,
    format: ObjectFormat,
) -> Result<Option<ResolvedRef>, GitError> {
    if refspec.is_empty() || refspec.contains('\0') {
        return Ok(None);
    }

    let kind = if refspec.starts_with(HEADS) {
        Some(RefKind::Branch)
    } else if refspec.starts_with(TAGS) {
        Some(RefKind::Tag)
    } else {
        None
    };

    if let Some(kind) = kind {
        // Reject names git itself would refuse before touching the ref store.
        if gix::refs::FullName::try_from(refspec).is_err() {
            return Ok(None);
        }
        let Some(mut reference) = repo
            .try_find_reference(refspec)
            .map_err(|e| repo_error("looking up ref", e))?
        else {
            return Ok(None);
        };
        if reference.name().as_bstr() != <&BStr>::from(refspec) {
            return Ok(None);
        }
        let Ok(id) = reference.peel_to_id() else {
            return Ok(None);
        };
        if !is_commit(repo, &id) {
            return Ok(None);
        }
        let name = reference.name();
        return Ok(Some(ResolvedRef {
            name: name.as_bstr().to_str_lossy().into_owned(),
            short: name.shorten().to_str_lossy().into_owned(),
            kind,
            oid: oid_from_gix(&id),
        }));
    }

    if !Oid::is_full_hex(refspec, format) {
        return Ok(None);
    }
    let Ok(id) = gix::ObjectId::from_hex(refspec.as_bytes()) else {
        return Ok(None);
    };
    let Some(object) = repo
        .try_find_object(id)
        .map_err(|e| repo_error("looking up object", e))?
    else {
        return Ok(None);
    };
    let commit = match object.kind {
        gix::objs::Kind::Commit => object.id,
        gix::objs::Kind::Tag => match object.peel_to_kind(gix::objs::Kind::Commit) {
            Ok(peeled) => peeled.id,
            Err(_) => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(commit_ref(oid_from_gix(&commit))))
}

/// Natural ("version") ordering: digit runs compare numerically, the rest
/// bytewise. Mirrors `git for-each-ref --sort=version:refname` closely enough
/// for tag lists.
pub(super) fn version_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.as_bytes();
    let mut b = b.as_bytes();
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&x), Some(&y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (na, rest_a) = split_digits(a);
                let (nb, rest_b) = split_digits(b);
                let na = na
                    .iter()
                    .skip_while(|&&d| d == b'0')
                    .copied()
                    .collect::<Vec<_>>();
                let nb = nb
                    .iter()
                    .skip_while(|&&d| d == b'0')
                    .copied()
                    .collect::<Vec<_>>();
                match na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb)) {
                    Ordering::Equal => {}
                    other => return other,
                }
                a = rest_a;
                b = rest_b;
            }
            (Some(&x), Some(&y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

fn split_digits(s: &[u8]) -> (&[u8], &[u8]) {
    let end = s
        .iter()
        .position(|d| !d.is_ascii_digit())
        .unwrap_or(s.len());
    s.split_at(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        let mut tags = vec![
            "v1.10.0",
            "v1.2.0",
            "v1.9.1",
            "v2.0.0",
            "v1.2.0-rc1",
            "beta",
            "v0.9",
        ];
        tags.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(
            tags,
            [
                "v2.0.0",
                "v1.10.0",
                "v1.9.1",
                "v1.2.0-rc1",
                "v1.2.0",
                "v0.9",
                "beta"
            ]
        );
        assert_eq!(version_cmp("a01", "a1"), Ordering::Equal);
        assert_eq!(version_cmp("a2", "a10"), Ordering::Less);
    }
}
