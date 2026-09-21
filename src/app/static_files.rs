//! `GET /_static/{file}`: the stylesheet and script, embedded in the binary.
//!
//! Both files are compiled in with `include_bytes!`, so the executable needs
//! nothing next to it at runtime. Their public names carry a content hash
//! (`app-<fnv1a64>.css`), computed once at startup, so the URLs change
//! whenever the files do and can be cached forever. The route serves exactly
//! these two names and answers 404 to everything else; it never touches the
//! file system.

use std::sync::LazyLock;

use topcoat::{
    Result,
    context::Cx,
    router::{
        Body,
        error::not_found,
        header::{CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS},
        path_param,
        response::Response,
        route,
    },
};

/// One embedded file: its source name under `static/`, its media type and
/// its bytes.
struct StaticFile {
    name: &'static str,
    content_type: &'static str,
    bytes: &'static [u8],
}

/// Everything the layout links to. Add a file here and it is served.
static FILES: [StaticFile; 2] = [
    StaticFile {
        name: "app.css",
        content_type: "text/css; charset=utf-8",
        bytes: include_bytes!("../../static/app.css"),
    },
    StaticFile {
        name: "app.js",
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../../static/app.js"),
    },
];

/// The content-hashed public names, in the order of [`FILES`].
static HASHED_NAMES: LazyLock<[String; 2]> = LazyLock::new(|| {
    std::array::from_fn(|index| {
        let file = &FILES[index];
        let (stem, extension) = file.name.rsplit_once('.').unwrap_or((file.name, ""));
        format!("{stem}-{:016x}.{extension}", fnv1a64(file.bytes))
    })
});

/// The root-relative URL of the embedded file `name` (e.g. `"app.css"`).
///
/// Panics when `name` is not one of the embedded files; callers pass literal
/// names, so this is a programming error caught by the tests.
pub fn static_url(name: &str) -> String {
    let index = FILES
        .iter()
        .position(|file| file.name == name)
        .unwrap_or_else(|| panic!("no embedded static file named {name:?}"));
    format!("/_static/{}", HASHED_NAMES[index])
}

// A single segment; `..`, `/` or a NUL never match an embedded name.
path_param!(file);

#[route(GET "/_static/{file}")]
pub async fn static_file(cx: &Cx) -> Result<Response> {
    let requested: &str = path_param::<File>(cx);
    let index = HASHED_NAMES
        .iter()
        .position(|hashed| hashed == requested)
        .ok_or_else(not_found)?;
    let file = &FILES[index];
    Ok(Response::builder()
        .header(CONTENT_TYPE, file.content_type)
        .header(CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(file.bytes))?)
}

/// 64-bit FNV-1a: tiny, dependency-free and plenty for a cache-busting name.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_matches_reference_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn urls_are_hashed_and_stable() {
        let css = static_url("app.css");
        let js = static_url("app.js");
        assert!(
            css.starts_with("/_static/app-") && css.ends_with(".css"),
            "{css}"
        );
        assert!(
            js.starts_with("/_static/app-") && js.ends_with(".js"),
            "{js}"
        );
        assert_ne!(css, js);
        assert_eq!(css, static_url("app.css"));
        assert_eq!(
            css,
            format!("/_static/app-{:016x}.css", fnv1a64(FILES[0].bytes))
        );
    }

    #[test]
    #[should_panic(expected = "no embedded static file")]
    fn unknown_name_is_a_programming_error() {
        static_url("nope.css");
    }
}
