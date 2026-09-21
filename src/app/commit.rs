//! `GET /commit/{oid}`: one commit with its message, metadata and the diff
//! against its first parent (or the empty tree).

use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{error::not_found, page, path_param},
    view::{View, component, view},
};

use super::{
    AppState,
    components::{CopySource, copy_button, time_ago},
    git_error,
    url::{blob_url, commit_url, tree_url},
};
use crate::{
    git::{CommitDetail, Diff, DiffFile, DiffStatus, Hunk, LineKind, Oid},
    l10n::{Lang, lang, tr},
    limits::{DIFF_MAX_BYTES, DIFF_MAX_FILES},
};

// The segment is validated by hand: only a full object id of the repository's
// hash length is accepted, abbreviations are not resolved.
path_param!(commit_id);

#[page("/commit/{commit_id}")]
pub async fn commit(cx: &Cx) -> Result<impl View> {
    let state = app_context::<AppState>(cx);
    let lang = lang(cx);
    let repo = &state.repo;
    let raw: &str = path_param::<CommitId>(cx);
    let oid = Oid::parse(raw, repo.object_format()).map_err(|_| not_found())?;

    let (detail, diff) = tokio::try_join!(repo.commit(&oid), repo.diff(&oid)).map_err(git_error)?;
    let Some(detail) = detail else {
        return Err(not_found().into());
    };

    Ok(view! {
        cx =>
        commit_header(detail: &detail)
        commit_meta(lang: lang, detail: &detail, diff: &diff)
        diff_summary(lang: lang, diff: &diff)
        for (index, file) in diff.files.iter().enumerate() {
            diff_file(lang: lang, index: index, file: file, commit_oid: &detail.oid)
        }
        if diff.truncated {
            <div class="notice">
                (tr!(
                    lang,
                    "commit.diff_truncated",
                    files = DIFF_MAX_FILES,
                    mib = DIFF_MAX_BYTES / (1024 * 1024),
                ))
            </div>
        }
    })
}

#[component]
async fn commit_header(detail: &CommitDetail) -> Result<impl View> {
    Ok(view! {
        <section class="commit-header">
            <h1 class="commit-header__title">(detail.title.clone())</h1>
            if !detail.body.is_empty() {
                <pre class="commit-header__body">(detail.body.clone())</pre>
            }
        </section>
    })
}

/// Full id, author, committer (when different), parents and a link to browse
/// the tree at this commit.
#[component]
async fn commit_meta(lang: Lang, detail: &CommitDetail, diff: &Diff) -> Result<impl View> {
    let oid = detail.oid.as_str();
    let author = format!("{} <{}>", detail.author_name, detail.author_email);
    let committer = format!("{} <{}>", detail.committer_name, detail.committer_email);
    let committer_differs = detail.committer_name != detail.author_name
        || detail.committer_email != detail.author_email
        || detail.committer_time != detail.author_time;
    let is_merge = detail.parents.len() > 1;
    let first_parent = diff
        .compared_against
        .as_ref()
        .map(|parent| parent.short().to_owned())
        .unwrap_or_default();
    let copy_label = tr!(lang, "action.copy_full_id");
    Ok(view! {
        <dl class="commit-meta">
            <dt>(tr!(lang, "commit.commit"))</dt>
            <dd>
                <span class="oid mono">(oid)</span>
                " "
                copy_button(
                    lang: lang,
                    source: CopySource::Text(oid),
                    label: &copy_label
                )
                " "
                <a class="btn btn--small" href=(tree_url(oid, ""))>
                    (tr!(lang, "action.browse_files"))
                </a>
            </dd>
            <dt>(tr!(lang, "commit.author"))</dt>
            <dd>
                (author)
                " · "
                time_ago(lang: lang, ts: detail.author_time)
            </dd>
            if committer_differs {
                <dt>(tr!(lang, "commit.committer"))</dt>
                <dd>
                    (committer)
                    " · "
                    time_ago(lang: lang, ts: detail.committer_time)
                </dd>
            }
            <dt>(tr!(lang, "commit.parents"))</dt>
            <dd>
                if detail.parents.is_empty() {
                    <span class="muted">(tr!(lang, "commit.root"))</span>
                }
                for parent in &detail.parents {
                    <a class="oid mono" href=(commit_url(parent.as_str()))>
                        (parent.short().to_owned())
                    </a>
                }
            </dd>
        </dl>
        if is_merge {
            <div class="notice">
                (tr!(lang, "commit.merge_notice.before"))
                " "
                <span class="oid mono">(first_parent)</span>
                (tr!(lang, "commit.merge_notice.after"))
            </div>
        }
    })
}

/// Human-readable status of a changed file.
fn status_label(lang: Lang, file: &DiffFile) -> String {
    match file.status {
        DiffStatus::Added => tr!(lang, "commit.status.added"),
        DiffStatus::Deleted => tr!(lang, "commit.status.deleted"),
        DiffStatus::Modified => tr!(lang, "commit.status.modified"),
        DiffStatus::Renamed { .. } => tr!(lang, "commit.status.renamed"),
        DiffStatus::Copied { .. } => tr!(lang, "commit.status.copied"),
        DiffStatus::ModeChanged => tr!(lang, "commit.status.mode_changed"),
    }
}

/// `old → new` for renames and copies, the single path otherwise.
fn path_label(file: &DiffFile) -> String {
    match (&file.status, &file.old_path, &file.new_path) {
        (DiffStatus::Renamed { .. } | DiffStatus::Copied { .. }, Some(old), Some(new)) => {
            format!("{} → {}", old.display(), new.display())
        }
        _ => file.path().display(),
    }
}

fn is_rename(file: &DiffFile) -> bool {
    matches!(
        file.status,
        DiffStatus::Renamed { .. } | DiffStatus::Copied { .. }
    )
}

/// `100644 → 100755` when the mode changed.
fn mode_label(file: &DiffFile) -> Option<String> {
    match (&file.old_mode, &file.new_mode) {
        (Some(old), Some(new)) if old != new => Some(format!("{old} → {new}")),
        _ => None,
    }
}

/// `+A` / `−D` counts, or a word when there are none to count.
#[component]
async fn file_stats(lang: Lang, file: &DiffFile) -> Result<impl View> {
    let counts = match (file.additions, file.deletions) {
        (Some(add), Some(del)) => Some((add, del)),
        _ => None,
    };
    let is_binary = file.is_binary;
    let mode_only = file.status == DiffStatus::ModeChanged;
    let renamed_as_is = is_rename(file) && file.hunks.is_empty();
    Ok(view! {
        if is_binary {
            <span class="muted">(tr!(lang, "commit.stat.binary"))</span>
        } else if mode_only {
            <span class="muted">(tr!(lang, "commit.stat.mode"))</span>
        } else if renamed_as_is {
            <span class="muted">(tr!(lang, "commit.stat.unchanged"))</span>
        } else if let Some((add, del)) = counts {
            <span class="stat-add">
                "+"
                (add)
            </span>
            <span class="stat-del">
                "−"
                (del)
            </span>
        }
    })
}

#[component]
async fn diff_summary(lang: Lang, diff: &Diff) -> Result<impl View> {
    let count = diff.files.len();
    let files_changed = if count == 1 {
        tr!(lang, "commit.files_changed.one", count = count)
    } else {
        tr!(lang, "commit.files_changed.other", count = count)
    };
    let has_counts = diff.files.iter().any(|f| f.additions.is_some());
    Ok(view! {
        <section class="diff-summary panel">
            <div class="panel__header">
                if count == 0 {
                    (tr!(lang, "commit.no_changes"))
                } else {
                    (files_changed)
                    if has_counts {
                        " "
                        <span class="diff-summary__stats">
                            <span class="stat-add">
                                "+"
                                (diff.additions)
                            </span>
                            <span class="stat-del">
                                "−"
                                (diff.deletions)
                            </span>
                        </span>
                    }
                }
            </div>
            if count > 0 {
                <ol>
                    for (index, file) in diff.files.iter().enumerate() {
                        <li>
                            <a
                                class="diff-summary__file"
                                href=(format!("#diff-{index}"))
                            >
                                (path_label(file))
                            </a>
                            <span class="badge">(status_label(lang, file))</span>
                            <span class="diff-summary__stats">
                                file_stats(lang: lang, file: file)
                            </span>
                        </li>
                    }
                </ol>
            }
        </section>
    })
}

#[component]
async fn diff_file(
    lang: Lang,
    index: usize,
    file: &DiffFile,
    commit_oid: &Oid,
) -> Result<impl View> {
    let mode = mode_label(file);
    // Link to the file as it is at this commit (deleted files have no such view).
    let view_href = match file.status {
        DiffStatus::Deleted => None,
        _ => file
            .new_path
            .as_ref()
            .and_then(|path| path.as_str())
            .map(|path| blob_url(commit_oid.as_str(), path)),
    };
    let is_binary = file.is_binary;
    let truncated = file.truncated;
    let mode_only = file.status == DiffStatus::ModeChanged;
    let renamed_as_is = is_rename(file) && file.hunks.is_empty();
    let no_hunks = file.hunks.is_empty();
    Ok(view! {
        <section class="diff-file" id=(format!("diff-{index}"))>
            <div class="diff-file__header">
                <span class="badge">(status_label(lang, file))</span>
                <span class="diff-file__path">(path_label(file))</span>
                if let Some(mode) = mode {
                    <span class="mono muted">(mode)</span>
                }
                <span class="diff-file__stats">file_stats(lang: lang, file: file)</span>
                if let Some(href) = view_href {
                    <a class="btn btn--small" href=(href)>
                        (tr!(lang, "action.view_file"))
                    </a>
                }
            </div>
            if is_binary {
                <div class="diff-file__binary">
                    (tr!(lang, "commit.binary_not_shown"))
                </div>
            } else if truncated {
                <div class="notice">(tr!(lang, "commit.diff_truncated_file"))</div>
            } else if mode_only {
                <div class="diff-file__binary">(tr!(lang, "commit.mode_only"))</div>
            } else if renamed_as_is {
                <div class="diff-file__binary">(tr!(lang, "commit.renamed_as_is"))</div>
            } else if no_hunks {
                <div class="diff-file__binary">
                    (tr!(lang, "commit.no_content_changes"))
                </div>
            } else {
                <div class="code-scroll">
                    <table class="diff">
                        <tbody>
                            for hunk in &file.hunks {
                                hunk_rows(hunk: hunk)
                            }
                        </tbody>
                    </table>
                </div>
            }
        </section>
    })
}

#[component]
async fn hunk_rows(hunk: &Hunk) -> Result<impl View> {
    Ok(view! {
        <tr class="diff__hunk"><td colspan="4">(hunk.header.clone())</td></tr>
        for line in &hunk.lines {
            let (class, sign) = match line.kind {
                LineKind::Add => ("diff__add", "+"),
                LineKind::Del => ("diff__del", "-"),
                LineKind::Context => ("diff__ctx", " "),
            };
            <tr class=(class)>
                <td class="diff__ln">
                    if let Some(no) = line.old_no {
                        (no)
                    }
                </td>
                <td class="diff__ln">
                    if let Some(no) = line.new_no {
                        (no)
                    }
                </td>
                <td class="diff__sign">(sign)</td>
                <td class="diff__code">(line.text.clone())</td>
            </tr>
        }
    })
}
