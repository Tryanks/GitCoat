//! `GET /blob?ref=&path=[&view=source]`: one file, shown as highlighted
//! source, rendered Markdown, an inline image or a binary/too-large panel.

use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{HeaderValue, StatusCode, header::LOCATION, page, query_params},
    view::{Unescaped, View, component, view},
};

use super::{
    AppState,
    components::{CopySource, copy_button, format_size, icon_submodule, icon_symlink},
    git_error,
    refpicker::ref_picker,
    url::{blob_url, raw_url, tree_url},
};
use crate::{
    git::{EntryKind, GitError, RefList, Repo, RepoPath, ResolvedRef, TreeEntry},
    l10n::{Lang, lang, tr},
    limits::{README_MAX_BYTES, TEXT_PREVIEW_MAX_BYTES, TEXT_PREVIEW_MAX_LINES},
    render::{
        content::{ContentKind, classify, decode_text},
        highlight::{Hint, highlight_lines, split_lines},
        markdown::{MarkdownCtx, render_markdown},
    },
};

#[query_params(error = bad_request)]
pub struct BlobQuery {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub path: Option<String>,
    pub view: Option<String>,
}

/// How a blob is presented, decided from its bytes and name.
enum Body {
    /// A gitlink: only the recorded commit id is known.
    Submodule,
    /// A symbolic link; `target` is the recorded link text, never followed.
    Symlink {
        target: String,
    },
    /// The object exceeds the read limit; only the raw download is offered.
    TooLarge,
    Empty,
    Image {
        mime: &'static str,
    },
    Binary,
    Text(TextView),
    /// Rendered Markdown (`shown_bytes` is `None` when nothing was cut).
    Markdown {
        html: String,
        shown_bytes: Option<usize>,
    },
}

/// Highlighted text ready for the code table.
struct TextView {
    /// One escaped HTML fragment per shown line.
    lines: Vec<String>,
    /// Bytes shown when the preview stops short of the file.
    shown_bytes: Option<usize>,
    /// Invalid UTF-8 was replaced.
    lossy: bool,
}

/// Everything the page shows for an existing file.
struct BlobData {
    resolved: ResolvedRef,
    path: RepoPath,
    refs: RefList,
    entry: TreeEntry,
    body: Body,
    source_view: bool,
}

enum Outcome {
    /// No such ref (or an empty repository).
    RefNotFound,
    /// The ref exists but `path` does not on it.
    PathNotFound {
        resolved: ResolvedRef,
        path: RepoPath,
    },
    /// `path` is a directory: send the browser to the tree view.
    Redirect(String),
    Show(Box<BlobData>),
}

#[page("/blob")]
pub async fn blob(cx: &Cx) -> Result<impl View> {
    let query = query_params::<BlobQuery>(cx)?;
    let state = app_context::<AppState>(cx);
    let lang = lang(cx);
    let repo = &state.repo;
    let path = RepoPath::parse(query.path.as_deref().unwrap_or("")).map_err(git_error)?;
    let source_view = query.view.as_deref() == Some("source");
    let ref_param = query.reference.as_deref().filter(|r| !r.is_empty());

    let resolved = match ref_param {
        Some(name) => repo.resolve(name).await.map_err(git_error)?,
        None => repo.default_ref().await.map_err(git_error)?,
    };
    let outcome = match resolved {
        None => Outcome::RefNotFound,
        Some(resolved) => match repo.entry(&resolved.oid, &path).await.map_err(git_error)? {
            None => Outcome::PathNotFound { resolved, path },
            Some(entry) if entry.kind == EntryKind::Dir => {
                Outcome::Redirect(tree_url(&resolved.name, path.as_str().unwrap_or("")))
            }
            Some(entry) => {
                let (refs, body) = tokio::try_join!(
                    repo.list_refs(),
                    load_body(repo, &resolved, &path, &entry, source_view)
                )
                .map_err(git_error)?;
                Outcome::Show(Box::new(BlobData {
                    resolved,
                    path,
                    refs,
                    entry,
                    body,
                    source_view,
                }))
            }
        },
    };
    let repo_name = state.config.repo_name();

    Ok(view! {
        cx =>
        match outcome {
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
            Outcome::PathNotFound { resolved, path } => {
                (StatusCode::NOT_FOUND)
                <section class="error-page">
                    <div class="error-page__code">"404"</div>
                    <h1>(tr!(lang, "error.file_not_found.title"))</h1>
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
            Outcome::Redirect(location) => {
                (StatusCode::FOUND)
                (HeaderValue::from_str(&location).ok().map(|value| (LOCATION, value)))
                <p>
                    (tr!(lang, "blob.is_directory"))
                    " "
                    <a href=(location.clone())>(tr!(lang, "blob.open_in_tree"))</a>
                </p>
            }
            Outcome::Show(data) => {
                let BlobData { resolved, path, refs, entry, body, source_view } = *data;
                let dir = path.parent().unwrap_or_else(RepoPath::root);
                let dir_str = dir.as_str().unwrap_or("").to_owned();
                <div class="toolbar">
                    <div class="toolbar__left">
                        ref_picker(
                            lang: lang,
                            refs: &refs,
                            resolved: &resolved,
                            link: |name: &str| tree_url(name, &dir_str)
                        )
                        breadcrumb(
                            lang: lang,
                            resolved: &resolved,
                            path: &path,
                            repo_name: &repo_name
                        )
                    </div>
                </div>
                blob_header(
                    lang: lang,
                    resolved: &resolved,
                    path: &path,
                    entry: &entry,
                    body: &body,
                    source_view: source_view
                )
                blob_body(
                    lang: lang,
                    resolved: &resolved,
                    path: &path,
                    entry: &entry,
                    body: &body
                )
            }
        }
    })
}

/// Read the blob behind `entry` and decide how to show it.
async fn load_body(
    repo: &Repo,
    resolved: &ResolvedRef,
    path: &RepoPath,
    entry: &TreeEntry,
    source_view: bool,
) -> std::result::Result<Body, GitError> {
    match entry.kind {
        EntryKind::Submodule => return Ok(Body::Submodule),
        EntryKind::Symlink => {
            let target = repo
                .read_blob(&entry.oid, TEXT_PREVIEW_MAX_BYTES)
                .await?
                .map(|(data, _)| String::from_utf8_lossy(&data).into_owned())
                .unwrap_or_default();
            return Ok(Body::Symlink { target });
        }
        EntryKind::Dir | EntryKind::File | EntryKind::Executable => {}
    }
    let read = repo
        .read_blob(&entry.oid, TEXT_PREVIEW_MAX_BYTES.max(README_MAX_BYTES))
        .await;
    let (data, cut) = match read {
        Ok(Some(read)) => read,
        Ok(None) => return Err(GitError::NotFound(format!("blob {}", entry.oid))),
        Err(GitError::Limit(_)) => return Ok(Body::TooLarge),
        Err(error) => return Err(error),
    };
    let size = entry.size.unwrap_or(data.len() as u64);
    let name = &entry.name_display;
    Ok(match classify(&data, size, name) {
        ContentKind::Empty => Body::Empty,
        ContentKind::Image { mime } => Body::Image { mime },
        ContentKind::Binary => Body::Binary,
        ContentKind::Markdown if !source_view => {
            let (text, cut) = preview_text(&data, cut, README_MAX_BYTES);
            let base_dir = path.parent().unwrap_or_else(RepoPath::root);
            let html = render_markdown(
                &text.0,
                &MarkdownCtx {
                    ref_name: resolved.oid.as_str(),
                    base_dir: &base_dir,
                },
            );
            Body::Markdown {
                html,
                shown_bytes: cut.then_some(text.0.len()),
            }
        }
        ContentKind::Markdown | ContentKind::Text { .. } => {
            let ((text, lossy), cut) = preview_text(&data, cut, TEXT_PREVIEW_MAX_BYTES);
            let mut lines = split_lines(&text);
            let too_many_lines = lines.len() > TEXT_PREVIEW_MAX_LINES;
            lines.truncate(TEXT_PREVIEW_MAX_LINES);
            let shown = lines.join("\n");
            let html = highlight_lines(&shown, Hint::for_file(name, &shown));
            Body::Text(TextView {
                lines: html,
                shown_bytes: (cut || too_many_lines).then_some(shown.len()),
                lossy,
            })
        }
    })
}

/// Decode at most `cap` bytes of `data` as text. Returns the decoded text
/// with its lossy flag, and whether the preview is shorter than the file.
fn preview_text(data: &[u8], cut: bool, cap: usize) -> ((String, bool), bool) {
    let cap = cap.min(data.len());
    let cut = cut || cap < data.len();
    (decode_text(trim_partial_utf8(&data[..cap], cut)), cut)
}

/// Drop a trailing incomplete UTF-8 sequence left by a byte cut, so the cut
/// itself does not read as invalid UTF-8.
fn trim_partial_utf8(data: &[u8], cut: bool) -> &[u8] {
    if !cut {
        return data;
    }
    match std::str::from_utf8(data) {
        Ok(_) => data,
        Err(error) if error.error_len().is_none() => &data[..error.valid_up_to()],
        Err(_) => data,
    }
}

/// Path breadcrumb: directories link to the tree, the file is current.
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
            <a href=(tree_url(&resolved.name, ""))>(repo_name)</a>
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

/// The type badge shown in the header.
fn type_badge(lang: Lang, entry: &TreeEntry, body: &Body) -> String {
    let executable = entry.kind == EntryKind::Executable;
    let key = match body {
        Body::Submodule => "blob.badge.submodule",
        Body::Symlink { .. } => "blob.badge.symlink",
        _ if executable => "blob.badge.executable",
        Body::Markdown { .. } => "blob.badge.markdown",
        Body::Image { .. } => "blob.badge.image",
        Body::Binary | Body::TooLarge => "blob.badge.binary",
        Body::Text(_) | Body::Empty => "blob.badge.text",
    };
    rust_i18n::t!(key, locale = lang.tag()).into_owned()
}

/// File name, size, line count, type badge and the action buttons.
#[component]
async fn blob_header(
    lang: Lang,
    resolved: &ResolvedRef,
    path: &RepoPath,
    entry: &TreeEntry,
    body: &Body,
    source_view: bool,
) -> Result<impl View> {
    let path_str = path.as_str().unwrap_or("").to_owned();
    let raw_href = raw_url(&resolved.name, &path_str);
    let permalink = blob_url(resolved.oid.as_str(), &path_str);
    let rendered_href = blob_url(&resolved.name, &path_str);
    let source_href = format!("{rendered_href}&view=source");
    let is_markdown = match body {
        Body::Markdown { .. } => true,
        Body::Text(_) => {
            source_view && crate::render::content::is_markdown_name(&entry.name_display)
        }
        _ => false,
    };
    let lines = match body {
        Body::Text(text) if text.shown_bytes.is_none() => Some(text.lines.len()),
        _ => None,
    };
    let badge = type_badge(lang, entry, body);
    let has_raw = !matches!(body, Body::Submodule);
    let permalink_label = tr!(lang, "action.permalink");
    let copy_path_label = tr!(lang, "action.copy_path");
    let lines_text = lines.map(|count| {
        if count == 1 {
            tr!(lang, "blob.lines.one", count = count)
        } else {
            tr!(lang, "blob.lines.other", count = count)
        }
    });
    Ok(view! {
        <div class="blob-header">
            <div class="blob-header__meta">
                <span class="blob-header__name mono">(entry.name_display.clone())</span>
                if let Some(size) = entry.size {
                    <span>(format_size(size))</span>
                }
                if let Some(lines_text) = lines_text {
                    <span>(lines_text)</span>
                }
                <span class="badge">(badge)</span>
            </div>
            <div class="blob-header__actions">
                if is_markdown {
                    <span
                        class="btn-group"
                        role="group"
                        aria-label=(tr!(lang, "action.view"))
                    >
                        <a
                            class="btn btn--small"
                            href=(rendered_href.clone())
                            aria-current=((!source_view).then_some("page"))
                        >
                            (tr!(lang, "action.rendered"))
                        </a>
                        <a
                            class="btn btn--small"
                            href=(source_href.clone())
                            aria-current=(source_view.then_some("page"))
                        >
                            (tr!(lang, "action.source"))
                        </a>
                    </span>
                }
                if has_raw {
                    <a class="btn btn--small" href=(raw_href.clone())>
                        (tr!(lang, "action.raw"))
                    </a>
                }
                copy_button(
                    lang: lang,
                    source: CopySource::Href(&permalink),
                    label: &permalink_label
                )
                copy_button(
                    lang: lang,
                    source: CopySource::Text(&path_str),
                    label: &copy_path_label
                )
            </div>
        </div>
    })
}

/// The content area below the header.
#[component]
async fn blob_body(
    lang: Lang,
    resolved: &ResolvedRef,
    path: &RepoPath,
    entry: &TreeEntry,
    body: &Body,
) -> Result<impl View> {
    let path_str = path.as_str().unwrap_or("");
    let raw_href = raw_url(&resolved.name, path_str);
    let size = entry.size.unwrap_or(0);
    let size_text = format_size(size);
    let name = entry.name_display.clone();
    Ok(view! {
        match body {
            Body::Submodule => {
                <div class="panel blob-binary">
                    icon_submodule()
                    " "
                    (tr!(lang, "blob.submodule_at"))
                    " "
                    <span class="oid mono">(entry.oid.as_str().to_owned())</span>
                </div>
            }
            Body::Symlink { target } => {
                <div class="panel blob-binary">
                    icon_symlink()
                    " "
                    (tr!(lang, "blob.symlink_to"))
                    " "
                    <code class="mono">(target.clone())</code>
                    <p class="muted">(tr!(lang, "blob.symlink_note"))</p>
                </div>
            }
            Body::TooLarge => {
                <div class="blob-binary">
                    (tr!(lang, "blob.too_large", size = size_text))
                    " "
                    <a class="btn btn--small" href=(raw_href.clone())>
                        (tr!(lang, "action.download"))
                    </a>
                </div>
            }
            Body::Empty => <div class="notice">(tr!(lang, "blob.empty"))</div>,
            Body::Image { mime } => {
                <div class="blob-image">
                    <img
                        class="blob-image__img"
                        src=(raw_href.clone())
                        alt=(name.clone())
                    >
                    <p class="muted">
                        (*mime)
                        " · "
                        (size_text.clone())
                    </p>
                </div>
            }
            Body::Binary => {
                <div class="blob-binary">
                    (tr!(lang, "blob.binary", size = size_text))
                    " "
                    <a class="btn btn--small" href=(raw_href.clone())>
                        (tr!(lang, "action.download"))
                    </a>
                </div>
            }
            Body::Markdown { html, shown_bytes } => {
                <div class="markdown-body">
                    (Unescaped::new_unchecked(html.clone()))
                </div>
                if let Some(shown) = shown_bytes {
                    <div class="notice">
                        (tr!(
                            lang,
                            "blob.truncated_bytes",
                            shown = format_size(*shown as u64),
                            total = size_text,
                        ))
                        " "
                        <a href=(raw_href.clone())>(tr!(lang, "action.view_raw"))</a>
                    </div>
                }
            }
            Body::Text(text) => {
                if text.lossy {
                    <div class="notice">(tr!(lang, "blob.not_utf8"))</div>
                }
                <div class="code-scroll">
                    <table class="code">
                        <tbody>
                            for (index, line) in text.lines.iter().enumerate() {
                                let number = index + 1;
                                let anchor = format!("L{number}");
                                <tr>
                                    <td class="code__ln">
                                        <a id=(anchor.clone()) href=(format!("#{anchor}"))>
                                            (number)
                                        </a>
                                    </td>
                                    <td class="code__line">
                                        (Unescaped::new_unchecked(line.clone()))
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>
                if let Some(shown) = text.shown_bytes {
                    <div class="notice">
                        (tr!(
                            lang,
                            "blob.truncated_lines",
                            lines = text.lines.len(),
                            shown = format_size(shown as u64),
                            total = size_text,
                        ))
                        " "
                        <a href=(raw_href.clone())>(tr!(lang, "action.view_raw"))</a>
                    </div>
                }
            }
        }
    })
}
