//! `GET /tree?ref=&path=`: directory listing with ref picker, breadcrumb,
//! latest commit and README.

use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{StatusCode, page, query_params},
    view::{Unescaped, View, component, view},
};

use super::{
    AppState,
    components::{
        CopySource, copy_button, format_size, icon_commit, icon_file, icon_folder, icon_repo,
        icon_submodule, icon_symlink, time_ago,
    },
    git_error,
    refpicker::ref_picker,
    url::{blob_url, commit_url, tree_url},
};
use crate::{
    git::{CommitDetail, EntryKind, Readme, RefList, RepoPath, ResolvedRef, TreeListing},
    l10n::{Lang, lang, tr},
    limits::TREE_MAX_ENTRIES,
    render::{
        content::is_markdown_name,
        markdown::{MarkdownCtx, render_markdown},
    },
};

#[query_params(error = bad_request)]
pub struct TreeQuery {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub path: Option<String>,
}

#[page("/tree")]
pub async fn tree(cx: &Cx) -> Result<impl View> {
    let query = query_params::<TreeQuery>(cx)?;
    render_tree(
        cx,
        query.reference.as_deref(),
        query.path.as_deref().unwrap_or(""),
    )
    .await
}

/// Everything the tree page needs, resolved once.
struct TreeData {
    resolved: ResolvedRef,
    path: RepoPath,
    refs: RefList,
    commit: Option<CommitDetail>,
    listing: TreeListing,
    readme: Option<Readme>,
}

enum Outcome {
    /// The repository has no commits.
    Empty,
    /// The `ref` parameter does not name a commit.
    RefNotFound,
    /// The ref exists but `path` does not on it.
    PathNotFound {
        resolved: ResolvedRef,
        path: RepoPath,
        refs: RefList,
    },
    Listing(Box<TreeData>),
}

/// Render the tree page for `ref_param` (default ref when `None`) and `path`.
pub async fn render_tree(cx: &Cx, ref_param: Option<&str>, path: &str) -> Result<impl View> {
    let state = app_context::<AppState>(cx);
    let lang = lang(cx);
    let repo = &state.repo;
    let path = RepoPath::parse(path).map_err(git_error)?;

    let resolved = match ref_param.filter(|r| !r.is_empty()) {
        Some(name) => repo.resolve(name).await.map_err(git_error)?,
        None => repo.default_ref().await.map_err(git_error)?,
    };
    let outcome = match resolved {
        None if ref_param.is_some_and(|r| !r.is_empty()) => Outcome::RefNotFound,
        None => Outcome::Empty,
        Some(resolved) => {
            let (refs, listing) =
                tokio::try_join!(repo.list_refs(), repo.ls_tree(&resolved.oid, &path))
                    .map_err(git_error)?;
            match listing {
                None => Outcome::PathNotFound {
                    resolved,
                    path,
                    refs,
                },
                Some(listing) => {
                    let (commit, readme) = tokio::try_join!(
                        repo.commit(&resolved.oid),
                        repo.readme(&resolved.oid, &path)
                    )
                    .map_err(git_error)?;
                    Outcome::Listing(Box::new(TreeData {
                        resolved,
                        path,
                        refs,
                        commit,
                        listing,
                        readme,
                    }))
                }
            }
        }
    };

    let repo_name = state.config.repo_name();
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
                        <a class="btn" href="/">(tr!(lang, "error.back_default"))</a>
                    </div>
                </section>
            }
            Outcome::PathNotFound { resolved, path, refs } => {
                (StatusCode::NOT_FOUND)
                toolbar(
                    lang: lang,
                    refs: &refs,
                    resolved: &resolved,
                    path: &path,
                    repo_name: &repo_name,
                    clone_url: clone_url.as_deref()
                )
                <section class="error-page">
                    <div class="error-page__code">"404"</div>
                    <h1>(tr!(lang, "error.path_not_found.title"))</h1>
                    <p>
                        (tr!(
                            lang,
                            "error.not_on_ref",
                            path = path.display(),
                            name = resolved.short,
                        ))
                    </p>
                    <div class="error-page__actions">
                        <a class="btn" href=(tree_url(&resolved.name, ""))>
                            (tr!(lang, "error.browse_root", name = resolved.short))
                        </a>
                    </div>
                </section>
            }
            Outcome::Listing(data) => {
                let TreeData { resolved, path, refs, commit, listing, readme } = *data;
                toolbar(
                    lang: lang,
                    refs: &refs,
                    resolved: &resolved,
                    path: &path,
                    repo_name: &repo_name,
                    clone_url: clone_url.as_deref()
                )
                if let Some(commit) = &commit {
                    commit_bar(lang: lang, commit: commit)
                }
                tree_table(
                    lang: lang,
                    resolved: &resolved,
                    path: &path,
                    listing: &listing
                )
                if let Some(readme) = &readme {
                    readme_panel(
                        lang: lang,
                        readme: readme,
                        resolved: &resolved,
                        dir: &path
                    )
                }
            }
        }
    })
}

/// Ref picker, breadcrumb and clone button.
#[component]
async fn toolbar(
    lang: Lang,
    refs: &RefList,
    resolved: &ResolvedRef,
    path: &RepoPath,
    repo_name: &str,
    clone_url: Option<&str>,
) -> Result<impl View> {
    let copy_label = tr!(lang, "action.copy_clone_url");
    Ok(view! {
        <div class="toolbar">
            <div class="toolbar__left">
                ref_picker(
                    lang: lang,
                    refs: refs,
                    resolved: resolved,
                    link: |name| tree_url(name, path.as_str().unwrap_or(""))
                )
                breadcrumb(
                    lang: lang,
                    resolved: resolved,
                    path: path,
                    repo_name: repo_name
                )
            </div>
            <div class="toolbar__right">
                if let Some(url) = clone_url {
                    <span class="clone-url mono muted">(url)</span>
                    copy_button(
                        lang: lang,
                        source: CopySource::Text(url),
                        label: &copy_label
                    )
                }
            </div>
        </div>
    })
}

#[component]
async fn breadcrumb(
    lang: Lang,
    resolved: &ResolvedRef,
    path: &RepoPath,
    repo_name: &str,
) -> Result<impl View> {
    let crumbs = path.ancestors();
    let last = crumbs.len().saturating_sub(1);
    Ok(view! {
        <nav class="breadcrumb" aria-label=(tr!(lang, "tree.path"))>
            if path.is_root() {
                <span class="breadcrumb__current">(repo_name)</span>
            } else {
                <a href=(tree_url(&resolved.name, ""))>(repo_name)</a>
            }
            for (index, (ancestor, segment)) in crumbs.iter().enumerate() {
                <span class="breadcrumb__sep" aria-hidden="true">"/"</span>
                if index == last {
                    <span class="breadcrumb__current">
                        (String::from_utf8_lossy(segment).into_owned())
                    </span>
                } else {
                    <a href=(tree_url(&resolved.name, ancestor.as_str().unwrap_or("")))>
                        (String::from_utf8_lossy(segment).into_owned())
                    </a>
                }
            }
        </nav>
    })
}

/// Summary of the commit the page is pinned to.
#[component]
async fn commit_bar(lang: Lang, commit: &CommitDetail) -> Result<impl View> {
    Ok(view! {
        <div class="commit-bar">
            <span class="commit-bar__icon">icon_commit()</span>
            <a
                class="commit-bar__title truncate"
                href=(commit_url(commit.oid.as_str()))
            >
                (commit.title.clone())
            </a>
            <span class="commit-bar__meta muted">
                <span class="commit-bar__author">(commit.author_name.clone())</span>
                " · "
                time_ago(lang: lang, ts: commit.author_time)
                " · "
                <a class="oid mono" href=(commit_url(commit.oid.as_str()))>
                    (commit.short.clone())
                </a>
            </span>
        </div>
    })
}

#[component]
async fn tree_table(
    lang: Lang,
    resolved: &ResolvedRef,
    path: &RepoPath,
    listing: &TreeListing,
) -> Result<impl View> {
    let parent = path.parent();
    Ok(view! {
        <div class="panel tree-panel">
            <table class="tree">
                <tbody>
                    if let Some(parent) = parent {
                        <tr class="tree__row tree__up">
                            <td class="tree__icon">icon_folder()</td>
                            <td class="tree__name" colspan="3">
                                <a
                                    href=(tree_url(
                                        &resolved.name,
                                        parent.as_str().unwrap_or(""),
                                    ))
                                >
                                    ".."
                                </a>
                            </td>
                        </tr>
                    }
                    for entry in &listing.entries {
                        let valid_name = entry.is_valid_utf8();
                        let badge = entry_badge(lang, entry.kind, valid_name);
                        let full_path = path
                            .join(&entry.name)
                            .ok()
                            .and_then(|p| p.as_str().map(str::to_owned));
                        <tr class="tree__row">
                            <td class="tree__icon">
                                match entry.kind {
                                    EntryKind::Dir => icon_folder(key: &entry.name),
                                    EntryKind::Symlink => icon_symlink(key: &entry.name),
                                    EntryKind::Submodule => icon_submodule(key: &entry.name),
                                    EntryKind::File | EntryKind::Executable => icon_file(
                                        key: &entry.name
                                    ),
                                }
                            </td>
                            <td class="tree__name">
                                match (valid_name, full_path, entry.kind) {
                                    (true, Some(full), EntryKind::Dir) => {
                                        <a href=(tree_url(&resolved.name, &full))>
                                            (entry.name_display.clone())
                                        </a>
                                    }
                                    (
                                        true,
                                        Some(full),
                                        EntryKind::File | EntryKind::Executable
                                        | EntryKind::Symlink,
                                    ) => {
                                        <a href=(blob_url(&resolved.name, &full))>
                                            (entry.name_display.clone())
                                        </a>
                                    }
                                    _ => <span>(entry.name_display.clone())</span>,
                                }
                            </td>
                            <td class="tree__meta muted">
                                if let Some(badge) = badge {
                                    <span class="badge">(badge)</span>
                                }
                                if entry.kind == EntryKind::Submodule {
                                    " "
                                    <span class="oid mono">
                                        (entry.oid.short().to_owned())
                                    </span>
                                }
                            </td>
                            <td class="tree__size muted">
                                if let Some(size) = entry.size {
                                    (format_size(size))
                                }
                            </td>
                        </tr>
                    }
                    if listing.entries.is_empty() {
                        <tr class="tree__row">
                            <td class="tree__name muted" colspan="4">
                                (tr!(lang, "tree.empty_dir"))
                            </td>
                        </tr>
                    }
                </tbody>
            </table>
            if listing.truncated {
                <div class="tree__truncated notice">
                    (tr!(lang, "tree.truncated", count = TREE_MAX_ENTRIES))
                </div>
            }
        </div>
    })
}

/// The badge next to an entry name, if it needs one.
fn entry_badge(lang: Lang, kind: EntryKind, valid_name: bool) -> Option<String> {
    match kind {
        EntryKind::Symlink => Some(tr!(lang, "tree.badge.symlink")),
        EntryKind::Executable => Some(tr!(lang, "tree.badge.executable")),
        EntryKind::Submodule => Some(tr!(lang, "tree.badge.submodule")),
        EntryKind::Dir | EntryKind::File if !valid_name => {
            Some(tr!(lang, "tree.badge.unsupported_name"))
        }
        EntryKind::Dir | EntryKind::File => None,
    }
}

/// README shown below the listing: `README.md`/`README.markdown` rendered as
/// Markdown (links pinned to the commit, resolved against `dir`), a plain
/// `README` as escaped preformatted text.
#[component]
async fn readme_panel(
    lang: Lang,
    readme: &Readme,
    resolved: &ResolvedRef,
    dir: &RepoPath,
) -> Result<impl View> {
    let text = String::from_utf8_lossy(&readme.data).into_owned();
    let is_markdown = is_markdown_name(&readme.entry.name_display);
    let html = is_markdown.then(|| {
        render_markdown(
            &text,
            &MarkdownCtx {
                ref_name: resolved.oid.as_str(),
                base_dir: dir,
            },
        )
    });
    let href = dir
        .join(&readme.entry.name)
        .ok()
        .and_then(|p| p.as_str().map(|full| blob_url(&resolved.name, full)));
    Ok(view! {
        <section class="readme panel">
            <div class="readme__header panel__header">
                icon_file()
                if let Some(href) = href {
                    <a class="mono" href=(href)>(readme.entry.name_display.clone())</a>
                } else {
                    <span class="mono">(readme.entry.name_display.clone())</span>
                }
            </div>
            <div class="markdown-body panel__body">
                if let Some(html) = html {
                    (Unescaped::new_unchecked(html))
                } else {
                    <pre>(text)</pre>
                }
                if readme.truncated {
                    <p class="notice">(tr!(lang, "tree.readme_truncated"))</p>
                }
            </div>
        </section>
    })
}

/// Shown when the repository has no commits.
#[component]
pub async fn empty_state(lang: Lang, clone_url: Option<&str>) -> Result<impl View> {
    let copy_label = tr!(lang, "action.copy_clone_url");
    Ok(view! {
        <section class="empty-state">
            <div class="empty-state__icon">icon_repo()</div>
            <h2>(tr!(lang, "tree.no_commits"))</h2>
            <p>(tr!(lang, "tree.empty_repo"))</p>
            if let Some(url) = clone_url {
                <p>
                    <code class="mono">(url)</code>
                    " "
                    copy_button(
                        lang: lang,
                        source: CopySource::Text(url),
                        label: &copy_label
                    )
                </p>
            }
        </section>
    })
}
