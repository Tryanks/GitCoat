//! Commit details and history walks.

use gix::bstr::ByteSlice;

use super::{
    CommitDetail, CommitSummary, GitError, Oid, Repo,
    convert::{oid_from_gix, oid_to_gix, repo_error, timestamp},
};
use crate::limits::COMMITS_MAX_SKIP;

impl Repo {
    /// Full details of a commit. `None` if `oid` is not a commit (annotated
    /// tags are peeled).
    pub async fn commit(&self, oid: &Oid) -> Result<Option<CommitDetail>, GitError> {
        let oid = oid.clone();
        self.run(move |repo| commit(repo, &oid)).await
    }

    /// History starting at `start` (newest first by commit time), skipping
    /// `skip` commits and returning up to `limit` plus whether more follow.
    ///
    /// `skip` beyond `COMMITS_MAX_SKIP` is refused with [`GitError::NotFound`]
    /// so a page number cannot trigger an unbounded walk.
    pub async fn log(
        &self,
        start: &Oid,
        skip: usize,
        limit: usize,
    ) -> Result<(Vec<CommitSummary>, bool), GitError> {
        let start = start.clone();
        self.run(move |repo| log(repo, &start, skip, limit)).await
    }
}

fn commit(repo: &gix::Repository, oid: &Oid) -> Result<Option<CommitDetail>, GitError> {
    let id = oid_to_gix(oid)?;
    let Some(object) = repo
        .try_find_object(id)
        .map_err(|e| repo_error("looking up commit", e))?
    else {
        return Ok(None);
    };
    let commit = match object.kind {
        gix::objs::Kind::Commit => object.into_commit(),
        gix::objs::Kind::Tag => match object.peel_to_kind(gix::objs::Kind::Commit) {
            Ok(peeled) => peeled.into_commit(),
            Err(_) => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(detail(&commit)?))
}

fn detail(commit: &gix::Commit<'_>) -> Result<CommitDetail, GitError> {
    let decoded = commit
        .decode()
        .map_err(|e| repo_error("decoding commit", e))?;
    let committer = decoded
        .committer()
        .map_err(|e| repo_error("decoding committer", e))?;
    let message = decoded.message();
    Ok(CommitDetail {
        summary: summary_from(commit, &decoded)?,
        body: message
            .body
            .map(|body| body.to_str_lossy().trim_end().to_owned())
            .unwrap_or_default(),
        committer_name: committer.name.to_str_lossy().into_owned(),
        committer_email: committer.email.to_str_lossy().into_owned(),
    })
}

fn summary_from(
    commit: &gix::Commit<'_>,
    decoded: &gix::objs::CommitRef<'_>,
) -> Result<CommitSummary, GitError> {
    let author = decoded
        .author()
        .map_err(|e| repo_error("decoding author", e))?;
    let committer = decoded
        .committer()
        .map_err(|e| repo_error("decoding committer", e))?;
    let oid = oid_from_gix(&commit.id);
    Ok(CommitSummary {
        short: oid.short().to_owned(),
        oid,
        title: decoded.message().summary().to_str_lossy().into_owned(),
        author_name: author.name.to_str_lossy().into_owned(),
        author_email: author.email.to_str_lossy().into_owned(),
        author_time: timestamp(&author),
        committer_time: timestamp(&committer),
        parents: decoded.parents().map(|id| oid_from_gix(&id)).collect(),
    })
}

fn log(
    repo: &gix::Repository,
    start: &Oid,
    skip: usize,
    limit: usize,
) -> Result<(Vec<CommitSummary>, bool), GitError> {
    if skip > COMMITS_MAX_SKIP {
        return Err(GitError::not_found(format!(
            "history offset {skip} is beyond the {COMMITS_MAX_SKIP} limit"
        )));
    }
    let id = oid_to_gix(start)?;
    match repo
        .try_find_header(id)
        .map_err(|e| repo_error("looking up commit", e))?
    {
        Some(header) if header.kind() == gix::objs::Kind::Commit => {}
        _ => return Err(GitError::not_found(format!("{start} is not a commit"))),
    }

    let order = gix::traverse::commit::simple::CommitTimeOrder::NewestFirst;
    let walk = repo
        .rev_walk([id])
        .sorting(gix::revision::walk::Sorting::ByCommitTime(order))
        .all()
        .map_err(|e| repo_error("starting history walk", e))?;

    let mut commits = Vec::with_capacity(limit);
    let mut has_more = false;
    // Hard cap on iterations: the page window plus one to detect `has_more`.
    for (index, info) in walk.enumerate().take(skip + limit + 1) {
        let info = info.map_err(|e| repo_error("walking history", e))?;
        if index < skip {
            continue;
        }
        if commits.len() == limit {
            has_more = true;
            break;
        }
        let commit = info.object().map_err(|e| repo_error("reading commit", e))?;
        let decoded = commit
            .decode()
            .map_err(|e| repo_error("decoding commit", e))?;
        commits.push(summary_from(&commit, &decoded)?);
    }
    Ok((commits, has_more))
}
