//! URL builders for the fixed route table.
//!
//! Every user-controlled value goes through `form_urlencoded`, so callers never
//! concatenate raw ref names or paths into a URL.

use form_urlencoded::Serializer;

fn query_url(path: &str, pairs: &[(&str, &str)]) -> String {
    let mut serializer = Serializer::new(String::new());
    let mut any = false;
    for (key, value) in pairs {
        if value.is_empty() {
            continue;
        }
        serializer.append_pair(key, value);
        any = true;
    }
    if any {
        format!("{path}?{}", serializer.finish())
    } else {
        path.to_owned()
    }
}

/// `/tree?ref=...&path=...` (root path and empty ref are omitted).
pub fn tree_url(ref_name: &str, path: &str) -> String {
    query_url("/tree", &[("ref", ref_name), ("path", path)])
}

/// `/blob?ref=...&path=...`.
pub fn blob_url(ref_name: &str, path: &str) -> String {
    query_url("/blob", &[("ref", ref_name), ("path", path)])
}

/// `/raw?ref=...&path=...`.
pub fn raw_url(ref_name: &str, path: &str) -> String {
    query_url("/raw", &[("ref", ref_name), ("path", path)])
}

/// `/commits?ref=...` (first page; the page itself links on with `at=`).
pub fn commits_url(ref_name: &str) -> String {
    query_url("/commits", &[("ref", ref_name)])
}

/// `/commits?ref=...&at=<oid>&page=N`.
pub fn commits_page_url(ref_name: &str, at: &str, page: usize) -> String {
    let page = page.to_string();
    query_url(
        "/commits",
        &[("ref", ref_name), ("at", at), ("page", &page)],
    )
}

/// `/lang?set=<tag>&back=<path>`: switch the UI language and return to `back`.
pub fn lang_url(tag: &str, back: &str) -> String {
    query_url("/lang", &[("set", tag), ("back", back)])
}

/// `/commit/<oid>`; `oid` must be a validated hex id.
pub fn commit_url(oid: &str) -> String {
    let mut out = String::from("/commit/");
    out.extend(form_urlencoded::byte_serialize(oid.as_bytes()));
    out
}

/// Read a query string into `(key, value)` pairs (decoded).
pub fn parse_query(query: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_refs_and_paths() {
        assert_eq!(
            tree_url("refs/heads/main", ""),
            "/tree?ref=refs%2Fheads%2Fmain"
        );
        assert_eq!(
            tree_url("refs/heads/feat/x", "src/a b.rs"),
            "/tree?ref=refs%2Fheads%2Ffeat%2Fx&path=src%2Fa+b.rs"
        );
        assert_eq!(blob_url("", "-x&y=z"), "/blob?path=-x%26y%3Dz");
        assert_eq!(
            raw_url("refs/tags/v1", "über.txt"),
            "/raw?ref=refs%2Ftags%2Fv1&path=%C3%BCber.txt"
        );
        assert_eq!(commits_url(""), "/commits");
        assert_eq!(
            commits_page_url("refs/heads/main", "abc", 2),
            "/commits?ref=refs%2Fheads%2Fmain&at=abc&page=2"
        );
        assert_eq!(commit_url("0123abcd"), "/commit/0123abcd");
        assert_eq!(
            lang_url("zh-CN", "/tree?ref=refs%2Fheads%2Fmain"),
            "/lang?set=zh-CN&back=%2Ftree%3Fref%3Drefs%252Fheads%252Fmain"
        );
    }

    #[test]
    fn parse_round_trips() {
        let url = tree_url("refs/heads/a+b", "x y/z");
        let query = url.split_once('?').unwrap().1;
        let pairs = parse_query(query);
        assert_eq!(pairs[0], ("ref".to_owned(), "refs/heads/a+b".to_owned()));
        assert_eq!(pairs[1], ("path".to_owned(), "x y/z".to_owned()));
    }
}
