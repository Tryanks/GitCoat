//! `GET /lang?set=<tag>&back=<path>`: pin the UI language in a cookie and
//! send the browser back where it came from.
//!
//! `set` must be a supported tag (400 otherwise). `back` must be a local,
//! root-relative path (`/...` but not `//...`, no scheme); anything else
//! falls back to `/`, so the endpoint cannot be used as an open redirect.

use topcoat::{
    Result,
    context::Cx,
    router::{
        Body, HeaderValue, StatusCode,
        error::bad_request,
        header::{CACHE_CONTROL, LOCATION, SET_COOKIE},
        query_params,
        response::Response,
        route,
    },
};

use crate::l10n::{Lang, cookie_value};

#[query_params(error = bad_request)]
pub struct LangQuery {
    pub set: Option<String>,
    pub back: Option<String>,
}

#[route(GET "/lang")]
pub async fn lang(cx: &Cx) -> Result<Response> {
    let query = query_params::<LangQuery>(cx)?;
    let lang = query
        .set
        .as_deref()
        .and_then(Lang::from_tag)
        .ok_or_else(|| bad_request("unsupported language"))?;
    let back = safe_back(query.back.as_deref());
    Ok(Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(LOCATION, HeaderValue::from_str(back)?)
        .header(SET_COOKIE, cookie_value(lang))
        .header(CACHE_CONTROL, "no-store")
        .body(Body::empty())?)
}

/// `back` when it is a plain root-relative path, `/` otherwise.
pub fn safe_back(back: Option<&str>) -> &str {
    match back {
        Some(back)
            if back.starts_with('/')
                && !back.starts_with("//")
                && !back.starts_with("/\\")
                && back.bytes().all(|b| b.is_ascii_graphic()) =>
        {
            back
        }
        _ => "/",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_is_validated() {
        assert_eq!(safe_back(Some("/")), "/");
        assert_eq!(
            safe_back(Some("/tree?ref=refs%2Fheads%2Fmain")),
            "/tree?ref=refs%2Fheads%2Fmain"
        );
        assert_eq!(safe_back(Some("/commit/abc#diff-1")), "/commit/abc#diff-1");
        assert_eq!(safe_back(None), "/");
        assert_eq!(safe_back(Some("")), "/");
        assert_eq!(safe_back(Some("tree")), "/");
        assert_eq!(safe_back(Some("//evil.com")), "/");
        assert_eq!(safe_back(Some("/\\evil.com")), "/");
        assert_eq!(safe_back(Some("https://evil.com/")), "/");
        assert_eq!(safe_back(Some("javascript:alert(1)")), "/");
        assert_eq!(safe_back(Some("/a b")), "/");
        assert_eq!(safe_back(Some("/a\r\nSet-Cookie: x=y")), "/");
    }
}
