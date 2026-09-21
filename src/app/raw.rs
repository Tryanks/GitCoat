//! `GET /raw?ref=&path=`: the bytes of a blob.
//!
//! Content types are decided from the bytes, never from the extension: raster
//! images are served inline with their sniffed type, text as
//! `text/plain` (HTML, SVG and scripts included, so nothing served here can
//! run in the site's origin) and everything else as an
//! `application/octet-stream` attachment. Every response carries
//! `X-Content-Type-Options: nosniff`.
//!
//! The body is held in memory: gitoxide has no streaming reader for loose and
//! packed objects, so reads are gated at `RAW_MAX_BYTES` instead (413 above).

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, StatusCode,
        error::not_found,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS},
        query_params,
        response::Response,
        route,
    },
};

use super::{AppState, git_error};
use crate::{
    git::{EntryKind, GitError, RefKind, RepoPath},
    l10n::{lang, tr},
    limits::RAW_MAX_BYTES,
    render::content::{looks_like_text, sniff_image},
};

#[query_params(error = bad_request)]
pub struct RawQuery {
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub path: Option<String>,
}

/// RFC 5987 `attr-char` complement: everything but unreserved characters.
const FILENAME_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[route(GET "/raw")]
pub async fn raw(cx: &Cx) -> Result<Response> {
    let query = query_params::<RawQuery>(cx)?;
    let state = app_context::<AppState>(cx);
    let repo = &state.repo;
    let path = RepoPath::parse(query.path.as_deref().unwrap_or("")).map_err(git_error)?;

    let resolved = match query.reference.as_deref().filter(|r| !r.is_empty()) {
        Some(name) => repo.resolve(name).await.map_err(git_error)?,
        None => repo.default_ref().await.map_err(git_error)?,
    };
    let Some(resolved) = resolved else {
        return Err(not_found().into());
    };
    let entry = repo
        .entry(&resolved.oid, &path)
        .await
        .map_err(git_error)?
        .filter(|entry| !matches!(entry.kind, EntryKind::Dir | EntryKind::Submodule))
        .ok_or_else(not_found)?;

    let data = match repo.read_blob(&entry.oid, RAW_MAX_BYTES).await {
        Ok(Some((data, _))) => data,
        Ok(None) => return Err(not_found().into()),
        Err(GitError::Limit(_)) => {
            let text = tr!(
                lang(cx),
                "raw.too_large",
                mib = RAW_MAX_BYTES / (1024 * 1024)
            );
            return plain(StatusCode::PAYLOAD_TOO_LARGE, format!("{text}\n"));
        }
        Err(error) => return Err(git_error(error)),
    };

    // A full object id names an immutable snapshot; ref names move.
    let cache = if resolved.kind == RefKind::Commit {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let filename = entry.name_display.clone();
    let (content_type, disposition) = if entry.kind == EntryKind::Symlink {
        (
            "text/plain; charset=utf-8",
            content_disposition("inline", &filename),
        )
    } else if let Some(mime) = sniff_image(&data) {
        (mime, content_disposition("inline", &filename))
    } else if looks_like_text(&data) {
        (
            "text/plain; charset=utf-8",
            content_disposition("inline", &filename),
        )
    } else {
        (
            "application/octet-stream",
            content_disposition("attachment", &filename),
        )
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_DISPOSITION, disposition)
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(CACHE_CONTROL, cache)
        .body(Body::from(data))?;
    Ok(response)
}

/// A plain-text response with the standard hardening headers.
fn plain(status: StatusCode, text: String) -> Result<Response> {
    Ok(Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(CACHE_CONTROL, "no-cache")
        .body(Body::from(text))?)
}

/// `inline; filename="ascii"; filename*=UTF-8''percent-encoded` with a safe
/// ASCII fallback for clients that ignore the encoded form.
fn content_disposition(kind: &str, filename: &str) -> String {
    let ascii: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded = utf8_percent_encode(filename, FILENAME_ENCODE);
    format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_encodes_names() {
        assert_eq!(
            content_disposition("inline", "a b.txt"),
            "inline; filename=\"a b.txt\"; filename*=UTF-8''a%20b.txt"
        );
        assert_eq!(
            content_disposition("attachment", "中文\"x.bin"),
            "attachment; filename=\"___x.bin\"; filename*=UTF-8''%E4%B8%AD%E6%96%87%22x.bin"
        );
    }
}
