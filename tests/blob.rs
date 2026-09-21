//! Blob page, raw endpoint and README rendering.

mod common;

use common::{
    fixtures::{TempRepo, big_text},
    get, test_app,
};
use gitcoat::app::url::{blob_url, raw_url, tree_url};
use http::StatusCode;

const MAIN: &str = "refs/heads/main";

#[tokio::test]
async fn text_blob_is_highlighted_with_line_anchors() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, &blob_url(MAIN, "src/main.rs")).await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;
    assert!(html.contains("class=\"blob-header\""), "header");
    assert!(html.contains("<table class=\"code\">"), "code table");
    assert!(
        html.contains("<a id=\"L1\" href=\"#L1\">1</a>"),
        "line anchors: {html}"
    );
    assert!(html.contains("id=\"L10\""), "10th line");
    assert!(
        html.contains("<span class=\"source rust\">"),
        "syntect classes"
    );
    assert!(
        html.contains("<span class=\"badge\">Text</span>"),
        "type badge"
    );
    let lines = std::str::from_utf8(common::fixtures::MAIN_RS)
        .unwrap()
        .lines()
        .count();
    assert!(
        html.contains(&format!("<span>{lines} lines</span>")),
        "line count {lines}: {html}"
    );
    assert!(html.contains("<span>1.0 KiB</span>"), "size: {html}");
    assert!(
        html.contains(&format!(
            "href=\"{}\"",
            raw_url(MAIN, "src/main.rs").replace('&', "&amp;")
        )),
        "raw link"
    );
    let head = repo.head_oid();
    assert!(
        html.contains(&format!(
            "data-copy-href=\"{}\"",
            blob_url(&head, "src/main.rs").replace('&', "&amp;")
        )),
        "permalink uses the full oid: {html}"
    );
    assert!(html.contains("data-copy=\"src/main.rs\""), "copy path");
    assert!(
        !html.contains("Rendered"),
        "no markdown toggle for source files"
    );
    assert!(html.contains("class=\"ref-picker\""), "ref picker present");
    assert!(
        html.contains(&format!(
            "href=\"{}\"",
            tree_url(MAIN, "src").replace('&', "&amp;")
        )),
        "breadcrumb links to the parent directory"
    );
}

#[tokio::test]
async fn markdown_blob_is_rendered_by_default() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let head = repo.head_oid();
    let response = get(&app, &blob_url(MAIN, "README.md")).await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;
    assert!(
        html.contains("<div class=\"markdown-body\">"),
        "rendered container"
    );
    assert!(
        html.contains("<h1 id=\"gitcoat-fixture\">GitCoat fixture</h1>"),
        "heading id: {html}"
    );
    assert!(html.contains("<table>"), "table");
    assert!(
        html.contains("<input checked=\"\" disabled=\"\" type=\"checkbox\">"),
        "task list"
    );
    assert!(
        html.contains(&format!(
            "href=\"/blob?ref={head}&amp;path=docs%2Fguide.md\""
        )),
        "relative link pinned to the oid: {html}"
    );
    assert!(
        html.contains(&format!(
            "<img src=\"/raw?ref={head}&amp;path=assets%2Flogo.png\" alt=\"logo\">"
        )),
        "image via raw: {html}"
    );
    assert!(!html.contains("outside.md"), "escaping link dropped");
    assert!(
        html.contains("<a rel=\"noopener\">up</a>"),
        "escaping link keeps its text"
    );
    assert!(html.contains("href=\"#features\""), "anchor kept");
    assert!(!html.contains("<script>alert"), "script stripped");
    assert!(!html.contains("alert(1)"), "script content gone: {html}");
    assert!(!html.contains("javascript:"), "javascript link dropped");
    assert!(
        html.contains("<code class=\"language-rust\">"),
        "fenced block"
    );
    assert!(
        html.contains("<span class=\"source rust\">"),
        "fenced block highlighted"
    );
    assert!(
        html.contains("<span class=\"badge\">Markdown</span>"),
        "badge"
    );
    assert!(
        html.contains("aria-current=\"page\"") && html.contains(">Rendered<"),
        "rendered toggle active"
    );
    assert!(html.contains("&amp;view=source\""), "source toggle link");
}

#[tokio::test]
async fn markdown_source_view_shows_code_table() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(
        &app,
        &format!("{}&view=source", blob_url(MAIN, "README.md")),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;
    assert!(
        html.contains("<table class=\"code\">"),
        "code table: {html}"
    );
    assert!(html.contains("GitCoat fixture"), "source text");
    assert!(
        !html.contains("<div class=\"markdown-body\">"),
        "not rendered"
    );
    assert!(html.contains("&lt;</span>"), "escaped source: {html}");
    assert!(!html.contains("<script>alert"), "never raw");
    assert!(html.contains(">Source<"), "toggle present");
}

#[tokio::test]
async fn home_readme_is_rendered_html() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let head = repo.head_oid();
    let html = get(&app, "/").await.body;
    assert!(html.contains("class=\"readme panel\""), "readme panel");
    assert!(
        html.contains("<h1 id=\"gitcoat-fixture\">GitCoat fixture</h1>"),
        "rendered heading"
    );
    assert!(
        !html.contains("<pre># GitCoat fixture"),
        "no longer preformatted"
    );
    assert!(
        html.contains(&format!(
            "href=\"/blob?ref={head}&amp;path=docs%2Fguide.md\""
        )),
        "links pinned to the oid: {html}"
    );
    assert!(
        html.contains(&format!(
            "src=\"/raw?ref={head}&amp;path=assets%2Flogo.png\""
        )),
        "image via raw"
    );
    assert!(!html.contains("<script>alert"), "script stripped");
    assert!(!html.contains("alert(1)"), "script content gone");
    assert!(!html.contains("javascript:"), "javascript dropped");
    assert!(
        html.contains(&format!(
            "<a class=\"mono\" href=\"{}\">README.md</a>",
            blob_url(MAIN, "README.md").replace('&', "&amp;")
        )),
        "header links to the blob: {html}"
    );
}

#[tokio::test]
async fn readme_in_subdirectory_resolves_relative_to_it() {
    let mut repo = TempRepo::rich();
    repo.commit_files(
        &[(
            "docs/README.md",
            b"[guide](GUIDE.md) [root](../README.md) ![l](../assets/logo.png)",
        )],
        "Add docs README",
    );
    let head = repo.head_oid();
    let app = test_app(&repo.path);
    let html = get(&app, &tree_url(MAIN, "docs")).await.body;
    assert!(
        html.contains(&format!(
            "href=\"/blob?ref={head}&amp;path=docs%2FGUIDE.md\""
        )),
        "{html}"
    );
    assert!(
        html.contains(&format!("href=\"/blob?ref={head}&amp;path=README.md\"")),
        "{html}"
    );
    assert!(
        html.contains(&format!(
            "src=\"/raw?ref={head}&amp;path=assets%2Flogo.png\""
        )),
        "{html}"
    );
}

#[tokio::test]
async fn html_and_svg_are_shown_and_served_as_text() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for (path, needle) in [("page.html", "xss"), ("icon.svg", "onload")] {
        let page = get(&app, &blob_url(MAIN, path)).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert!(
            page.body.contains("<table class=\"code\">"),
            "{path} shown as source"
        );
        assert!(page.body.contains(needle), "{path}: {}", page.body);
        assert!(page.body.contains("&lt;"), "{path} escaped");
        assert!(
            !page.body.contains("<script>alert"),
            "{path} never emits raw script"
        );
        assert!(
            !page.body.contains("<svg xmlns"),
            "{path} never emits raw svg"
        );

        let raw = get(&app, &raw_url(MAIN, path)).await;
        assert_eq!(raw.status, StatusCode::OK, "{path}");
        assert_eq!(raw.content_type(), "text/plain; charset=utf-8", "{path}");
        assert_eq!(raw.header("x-content-type-options"), "nosniff");
        assert!(
            raw.header("content-disposition").starts_with("inline;"),
            "{path}"
        );
        assert_eq!(raw.header("cache-control"), "no-cache");
    }
    let raw = get(&app, &raw_url(MAIN, "page.html")).await;
    assert_eq!(raw.bytes, common::fixtures::PAGE_HTML);
}

#[tokio::test]
async fn png_is_previewed_inline() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "assets/logo.png")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("<div class=\"blob-image\">"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains(&format!(
            "<img class=\"blob-image__img\" src=\"{}\" alt=\"logo.png\">",
            raw_url(MAIN, "assets/logo.png").replace('&', "&amp;")
        )),
        "{}",
        page.body
    );
    assert!(page.body.contains("<span class=\"badge\">Image</span>"));

    let raw = get(&app, &raw_url(MAIN, "assets/logo.png")).await;
    assert_eq!(raw.status, StatusCode::OK);
    assert_eq!(raw.content_type(), "image/png");
    assert!(raw.header("content-disposition").starts_with("inline;"));
    assert_eq!(raw.header("x-content-type-options"), "nosniff");
    assert_eq!(raw.bytes, common::fixtures::PNG_1X1);
}

#[tokio::test]
async fn binary_blob_gets_panel_and_attachment() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "bin/blob.bin")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("class=\"blob-binary\""), "{}", page.body);
    assert!(page.body.contains("Binary file · 32 B"), "{}", page.body);
    assert!(page.body.contains(">Download</a>"));
    assert!(page.body.contains("<span class=\"badge\">Binary</span>"));
    assert!(!page.body.contains("<table class=\"code\">"));

    let raw = get(&app, &raw_url(MAIN, "bin/blob.bin")).await;
    assert_eq!(raw.status, StatusCode::OK);
    assert_eq!(raw.content_type(), "application/octet-stream");
    assert_eq!(
        raw.header("content-disposition"),
        "attachment; filename=\"blob.bin\"; filename*=UTF-8''blob.bin"
    );
    assert_eq!(raw.bytes, common::fixtures::BLOB_BIN);
}

#[tokio::test]
async fn empty_file_shows_notice() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "empty.txt")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body
            .contains("<div class=\"notice\">This file is empty.</div>"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("<table class=\"code\">"));
    let raw = get(&app, &raw_url(MAIN, "empty.txt")).await;
    assert_eq!(raw.status, StatusCode::OK);
    assert!(raw.bytes.is_empty());
}

#[tokio::test]
async fn big_text_is_truncated_but_raw_is_complete() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "big.txt")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("Preview truncated"),
        "{}",
        &page.body[..2000]
    );
    assert!(
        page.body.contains("line 0000001: the quick brown fox"),
        "first line present"
    );
    assert!(page.body.contains("id=\"L10000\""), "10,000 lines shown");
    assert!(
        !page.body.contains("id=\"L10001\""),
        "not more than the line cap"
    );
    assert!(page.body.contains("of 1.5 MiB"), "full size mentioned");

    let raw = get(&app, &raw_url(MAIN, "big.txt")).await;
    assert_eq!(raw.status, StatusCode::OK);
    let expected = big_text();
    assert_eq!(expected.len(), 1_572_902);
    assert_eq!(raw.bytes.len(), 1_572_902);
    assert_eq!(raw.bytes, expected);
}

#[tokio::test]
async fn symlink_shows_target_not_content() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "link")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Symbolic link to"), "{}", page.body);
    assert!(
        page.body
            .contains("<code class=\"mono\">src/main.rs</code>"),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("HashMap"),
        "target file content is not shown"
    );
    assert!(page.body.contains("<span class=\"badge\">Symlink</span>"));

    let raw = get(&app, &raw_url(MAIN, "link")).await;
    assert_eq!(raw.status, StatusCode::OK);
    assert_eq!(raw.content_type(), "text/plain; charset=utf-8");
    assert_eq!(raw.body, "src/main.rs");
}

#[tokio::test]
async fn special_file_names_open_via_encoded_urls() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for (path, content) in [
        ("dir with spaces/中文文件.txt", "中文内容"),
        ("-leading-dash.txt", "leading dash"),
        ("weird#name%.txt", "weird name"),
    ] {
        let url = blob_url(MAIN, path);
        let page = get(&app, &url).await;
        assert_eq!(page.status, StatusCode::OK, "{url}");
        assert!(page.body.contains(content), "{url}: {}", page.body);
        let raw = get(&app, &raw_url(MAIN, path)).await;
        assert_eq!(raw.status, StatusCode::OK, "{url}");
        assert!(raw.body.starts_with(content));
    }
    let raw = get(&app, &raw_url(MAIN, "dir with spaces/中文文件.txt")).await;
    assert_eq!(
        raw.header("content-disposition"),
        "inline; filename=\"____.txt\"; filename*=UTF-8''%E4%B8%AD%E6%96%87%E6%96%87%E4%BB%B6.txt"
    );
}

#[tokio::test]
async fn directory_redirects_to_tree() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, &blob_url(MAIN, "src")).await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location"), tree_url(MAIN, "src"));
    let response = get(&app, &blob_url(MAIN, "")).await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location"), tree_url(MAIN, ""));
    let raw = get(&app, &raw_url(MAIN, "src")).await;
    assert_eq!(raw.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_file_is_404_with_link_back() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, &blob_url(MAIN, "src/nope.rs")).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(response.body.contains("File not found"));
    assert!(
        response
            .body
            .contains(&format!("href=\"{}\"", tree_url(MAIN, "")))
    );
    let raw = get(&app, &raw_url(MAIN, "src/nope.rs")).await;
    assert_eq!(raw.status, StatusCode::NOT_FOUND);
    let response = get(&app, &blob_url("refs/heads/nope", "README.md")).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(response.body.contains("Ref not found"));
    let raw = get(&app, &raw_url("refs/heads/nope", "README.md")).await;
    assert_eq!(raw.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn blob_at_tag_and_at_oid() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    // v1.0.0 predates page.html; docs/guide.md exists there under its old name.
    let at_tag = get(&app, &blob_url("refs/tags/v1.0.0", "docs/guide.md")).await;
    assert_eq!(at_tag.status, StatusCode::OK);
    assert!(
        at_tag.body.contains("<h1 id=\"guide\">Guide</h1>"),
        "{}",
        at_tag.body
    );
    assert!(
        at_tag.body.contains(&format!(
            "href=\"/blob?ref={}&amp;path=README.md\"",
            repo.v1_oid
        )),
        "links pinned to the tag's commit: {}",
        at_tag.body
    );
    let missing = get(&app, &blob_url("refs/tags/v1.0.0", "page.html")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let at_oid = get(&app, &blob_url(&repo.root_oid, "src/main.rs")).await;
    assert_eq!(at_oid.status, StatusCode::OK);
    assert!(at_oid.body.contains("<span class=\"source rust\">"));

    let raw_tag = get(&app, &raw_url("refs/tags/v1.0.0", "src/main.rs")).await;
    assert_eq!(raw_tag.header("cache-control"), "no-cache");
    let raw_oid = get(&app, &raw_url(&repo.root_oid, "src/main.rs")).await;
    assert_eq!(raw_oid.status, StatusCode::OK);
    assert_eq!(
        raw_oid.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(raw_oid.bytes, common::fixtures::MAIN_RS);
}

#[tokio::test]
async fn path_traversal_is_rejected() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for path in [
        "../../etc/passwd",
        "/etc/passwd",
        "src/../README.md",
        "./README.md",
    ] {
        for url in [blob_url(MAIN, path), raw_url(MAIN, path)] {
            let response = get(&app, &url).await;
            assert!(
                matches!(
                    response.status,
                    StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
                ),
                "{url}: {}",
                response.status
            );
            assert!(!response.body.contains("root:"), "{url} leaked content");
        }
    }
    let response = get(&app, "/raw?ref=HEAD&path=README.md").await;
    assert!(matches!(
        response.status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
    ));
}

#[tokio::test]
async fn uncommitted_changes_are_invisible() {
    let repo = TempRepo::rich().with_uncommitted_changes();
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "README.md")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(!page.body.contains("UNCOMMITTED CHANGE"));
    let raw = get(&app, &raw_url(MAIN, "README.md")).await;
    assert_eq!(raw.bytes, common::fixtures::README_MD);
    let home = get(&app, "/").await.body;
    assert!(!home.contains("UNCOMMITTED CHANGE"));
    assert!(!home.contains("UNCOMMITTED.txt"));
    let missing = get(&app, &blob_url(MAIN, "UNCOMMITTED.txt")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bare_repository_serves_blobs_too() {
    let repo = TempRepo::rich_bare();
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "src/main.rs")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("<span class=\"source rust\">"));
    let readme = get(&app, &blob_url(MAIN, "README.md")).await;
    assert!(readme.body.contains("<h1 id=\"gitcoat-fixture\">"));
    let raw = get(&app, &raw_url(MAIN, "assets/logo.png")).await;
    assert_eq!(raw.content_type(), "image/png");
}

#[tokio::test]
async fn executable_badge_and_lossy_utf8_notice() {
    let mut repo = TempRepo::rich();
    repo.commit_files(&[("latin1.txt", b"caf\xe9\n")], "Add latin-1 file");
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "scripts/run.sh")).await;
    assert!(
        page.body
            .contains("<span class=\"badge\">Executable</span>"),
        "{}",
        page.body
    );
    let page = get(&app, &blob_url(MAIN, "latin1.txt")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Not valid UTF-8"), "{}", page.body);
    assert!(page.body.contains("caf\u{fffd}"));
}

#[tokio::test]
async fn oversized_blob_is_refused_gracefully() {
    let mut repo = TempRepo::rich();
    // One byte over the raw read gate; text so the page would otherwise try to preview it.
    let huge = vec![b'a'; gitcoat::limits::RAW_MAX_BYTES + 1];
    repo.commit_files(&[("huge.txt", &huge)], "Add huge.txt");
    let app = test_app(&repo.path);
    let page = get(&app, &blob_url(MAIN, "huge.txt")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("too large to preview"), "{}", page.body);
    assert!(page.body.contains(">Download</a>"));
    let raw = get(&app, &raw_url(MAIN, "huge.txt")).await;
    assert_eq!(raw.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(raw.content_type(), "text/plain; charset=utf-8");
    assert!(raw.body.contains("64 MiB"));
}

/// Not a test: materializes the rich fixture under `GITCOAT_FIXTURE_DIR` for
/// manual runs of the binary (`cargo test --test blob -- --ignored fixture_repo`).
#[test]
#[ignore]
fn fixture_repo() {
    let target = std::env::var("GITCOAT_FIXTURE_DIR").expect("set GITCOAT_FIXTURE_DIR");
    let repo = TempRepo::rich();
    let _ = std::fs::remove_dir_all(&target);
    let status = std::process::Command::new("cp")
        .args(["-R", &repo.path.to_string_lossy(), &target])
        .status()
        .expect("copy fixture");
    assert!(status.success());
    println!("fixture repository copied to {target}");
}
