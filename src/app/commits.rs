//! `GET /commits?ref=&at=&page=`: paged history of a ref.
//!
//! `at` pins the commit the history starts from, so every pager link stays
//! valid even when the branch moves between requests. The first page (no
//! `at`) resolves `ref` and links onward with `at=<oid>`.

use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        StatusCode,
        error::{bad_request, not_found},
        page, query_params,
    },
    view::{View, component, view},
};

use super::{
    AppState,
    components::{icon_branch, icon_commit, icon_tag, time_ago},
    git_error,
    refpicker::ref_picker,
    tree::empty_state,
    url::{commit_url, commits_page_url, commits_url},
};
use crate::{
    git::{CommitSummary, Oid, RefKind, RefList, ResolvedRef},
    l10n::{Lang, lang, tr},
    limits::COMMITS_PER_PAGE,
};

#[query_params(error = bad_request)]
pub struct CommitsQuery {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub at: Option<String>,
    pub page: Option<usize>,
}

/// Everything a history page needs, resolved once.
struct HistoryData {
    resolved: ResolvedRef,
    refs: RefList,
    /// The pinned start commit every pager link carries.
    at: Oid,
    page: usize,
    commits: Vec<CommitSummary>,
    has_more: bool,
}

enum Outcome {
    /// The repository has no commits.
    Empty,
    /// The `ref` parameter does not name a commit.
    RefNotFound,
    History(Box<HistoryData>),
}

#[page("/commits")]
pub async fn commits(cx: &Cx) -> Result<impl View> {
    let state = app_context::<AppState>(cx);
    let lang = lang(cx);
    let repo = &state.repo;
    let query = query_params::<CommitsQuery>(cx)?;

    let page = query.page.unwrap_or(1);
    if page == 0 {
        return Err(bad_request("page must be 1 or greater").into());
    }
    let skip = (page - 1).saturating_mul(COMMITS_PER_PAGE);

    let ref_param = query.reference.as_deref().filter(|r| !r.is_empty());
    let resolved = match ref_param {
        Some(name) => repo.resolve(name).await.map_err(git_error)?,
        None => repo.default_ref().await.map_err(git_error)?,
    };
    let outcome = match resolved {
        None if ref_param.is_some() => Outcome::RefNotFound,
        None => Outcome::Empty,
        Some(resolved) => {
            // `at` must be a full object id; `log` then checks it is a commit.
            let at = match query.at.as_deref().filter(|at| !at.is_empty()) {
                Some(at) => Oid::parse(at, repo.object_format()).map_err(|_| not_found())?,
                None => resolved.oid.clone(),
            };
            let (refs, (commits, has_more)) =
                tokio::try_join!(repo.list_refs(), repo.log(&at, skip, COMMITS_PER_PAGE))
                    .map_err(git_error)?;
            Outcome::History(Box::new(HistoryData {
                resolved,
                refs,
                at,
                page,
                commits,
                has_more,
            }))
        }
    };

    let clone_url = state
        .config
        .clone_url
        .clone()
        .filter(|url| !url.trim().is_empty());

    Ok(view! {
        cx =>
        match outcome {
            Outcome::Empty => empty_state(lang: lang, clone_url: clone_url.as_deref()),
            Outcome::RefNotFound => {
                (StatusCode::NOT_FOUND)
                <section class="error-page">
                    <div class="error-page__code">"404"</div>
                    <h1>(tr!(lang, "error.ref_not_found.title"))</h1>
                    <p>(tr!(lang, "error.ref_not_found.text"))</p>
                    <div class="error-page__actions">
                        <a class="btn" href=(commits_url(""))>
                            (tr!(lang, "commits.history_default"))
                        </a>
                    </div>
                </section>
            }
            Outcome::History(data) => {
                let HistoryData { resolved, refs, at, page, commits, has_more } = *data;
                let past_end = commits.is_empty() && page > 1;
                <div class="toolbar">
                    <div class="toolbar__left">
                        ref_picker(
                            lang: lang,
                            refs: &refs,
                            resolved: &resolved,
                            link: commits_url
                        )
                    </div>
                </div>
                <h1 class="commits-heading">
                    match resolved.kind {
                        RefKind::Branch => icon_branch(),
                        RefKind::Tag => icon_tag(),
                        RefKind::Commit => icon_commit(),
                    }
                    " "
                    (tr!(lang, "commits.heading", name = resolved.short))
                </h1>
                if past_end {
                    (StatusCode::NOT_FOUND)
                    <section class="empty-state">
                        <div class="empty-state__icon">icon_commit()</div>
                        <h2>(tr!(lang, "commits.none_on_page"))</h2>
                        <p>
                            (tr!(
                                lang,
                                "commits.ends_before",
                                name = resolved.short,
                                page = page,
                            ))
                        </p>
                        <p>
                            <a
                                class="btn"
                                href=(commits_page_url(&resolved.name, at.as_str(), 1))
                            >
                                (tr!(lang, "commits.back_to_first"))
                            </a>
                        </p>
                    </section>
                } else {
                    commit_list(lang: lang, items: &commits)
                    pager(
                        lang: lang,
                        ref_name: &resolved.name,
                        at: at.as_str(),
                        page: page,
                        has_more: has_more
                    )
                }
            }
        }
    })
}

#[component]
async fn commit_list(lang: Lang, items: &[CommitSummary]) -> Result<impl View> {
    Ok(view! {
        <ol class="commits">
            for commit in items {
                let href = commit_url(commit.oid.as_str());
                let is_merge = commit.parents.len() > 1;
                <li class="commit-row">
                    <span class="commit-row__title">
                        <a href=(href.clone())>(commit.title.clone())</a>
                    </span>
                    <a class="oid mono" href=(href)>(commit.short.clone())</a>
                    <span class="commit-row__meta">
                        <strong>(commit.author_name.clone())</strong>
                        " "
                        (tr!(lang, "commits.committed"))
                        " "
                        time_ago(lang: lang, ts: commit.author_time)
                        if is_merge {
                            <span class="badge">(tr!(lang, "commits.merge"))</span>
                        }
                    </span>
                </li>
            }
        </ol>
    })
}

/// Newer / Older links; a disabled end is a `span` so it stays visible.
#[component]
async fn pager(
    lang: Lang,
    ref_name: &str,
    at: &str,
    page: usize,
    has_more: bool,
) -> Result<impl View> {
    let newer = (page > 1).then(|| commits_page_url(ref_name, at, page - 1));
    let older = has_more.then(|| commits_page_url(ref_name, at, page + 1));
    let newer_label = tr!(lang, "commits.newer");
    let older_label = tr!(lang, "commits.older");
    Ok(view! {
        <nav class="pager" aria-label=(tr!(lang, "commits.pagination"))>
            match newer {
                Some(href) => <a class="btn" href=(href) rel="prev">(newer_label)</a>,
                None => <span class="btn" aria-disabled="true">(newer_label)</span>,
            }
            <span class="pager__page muted">
                (tr!(lang, "commits.page", page = page))
            </span>
            match older {
                Some(href) => <a class="btn" href=(href) rel="next">(older_label)</a>,
                None => <span class="btn" aria-disabled="true">(older_label)</span>,
            }
        </nav>
    })
}
