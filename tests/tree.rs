//! Tree page, home page, error pages and the health check.

mod common;

use common::{
    fixtures::TempRepo, get, script_src, stylesheet_href, test_app, test_app_with, test_config,
};
use gitcoat::app::url::tree_url;
use http::StatusCode;

#[tokio::test]
async fn home_renders_repo_name_and_file_list() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, "/").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(
        response.content_type().starts_with("text/html"),
        "{}",
        response.content_type()
    );
    let html = &response.body;

    assert!(html.contains("<title>repo · GitCoat</title>"), "title");
    assert!(
        html.contains("class=\"repo-title__name\">repo<"),
        "repo name in top bar"
    );
    assert!(html.contains("A test repository"), "description");
    assert!(html.contains("git@example.com:test/repo.git"), "clone url");
    assert!(
        html.contains("data-copy=\"git@example.com:test/repo.git\""),
        "copy button"
    );
    assert!(
        html.contains("class=\"tab\" href=\"/\" aria-current=\"page\""),
        "Code tab is active"
    );

    // Directories first, then files; links go through /tree and /blob.
    let src = html
        .find("href=\"/tree?ref=refs%2Fheads%2Fmain&amp;path=src\"")
        .expect("src dir link");
    let readme = html
        .find("href=\"/blob?ref=refs%2Fheads%2Fmain&amp;path=README.md\"")
        .expect("README link");
    assert!(src < readme, "directories are listed before files");
    assert!(
        html.contains("path=dir+with+spaces\""),
        "space in dir name is encoded"
    );
    assert!(
        html.contains("path=-leading-dash.txt\""),
        "leading dash file is linked"
    );
    assert!(
        html.contains("path=weird%23name%25.txt\""),
        "# and % are encoded"
    );
    assert!(html.contains("class=\"badge\">symlink<"), "symlink badge");
    assert!(
        html.contains("class=\"ref-picker__item\" data-kind=\"branch\""),
        "ref picker items"
    );
    assert!(
        html.contains("aria-selected=\"true\""),
        "current ref selected in picker"
    );
    assert!(
        html.contains("data-kind=\"tag\" href=\"/tree?ref=refs%2Ftags%2Fv1.0.0\""),
        "tag links"
    );
    assert!(html.contains("class=\"commit-bar\""), "latest commit bar");
    assert!(html.contains("class=\"readme panel\""), "README panel");
    assert!(
        html.contains("<h1 id=\"gitcoat-fixture\">GitCoat fixture</h1>"),
        "README rendered as Markdown"
    );
    assert!(
        html.contains("<footer class=\"footer container\">"),
        "footer"
    );
    assert!(
        html.contains("localStorage.getItem('gitcoat-theme')"),
        "pre-paint theme script"
    );
}

#[tokio::test]
async fn embedded_static_files_are_served_with_immutable_caching() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    let href = stylesheet_href(&home.body).expect("stylesheet link");
    assert!(
        href.starts_with("/_static/app-") && href.ends_with(".css"),
        "{href}"
    );
    let src = script_src(&home.body).expect("script src");
    assert!(
        src.starts_with("/_static/app-") && src.ends_with(".js"),
        "{src}"
    );
    assert!(
        home.body
            .contains(&format!("<script src=\"{src}\" defer=\"\">"))
    );

    let css = get(&app, &href).await;
    assert_eq!(css.status, StatusCode::OK);
    assert!(
        css.content_type().starts_with("text/css"),
        "{}",
        css.content_type()
    );
    assert_eq!(
        css.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(css.header("x-content-type-options"), "nosniff");
    assert_eq!(css.bytes, include_bytes!("../static/app.css"));

    let js = get(&app, &src).await;
    assert_eq!(js.status, StatusCode::OK);
    assert!(
        js.content_type().starts_with("text/javascript"),
        "{}",
        js.content_type()
    );
    assert_eq!(
        js.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(js.bytes, include_bytes!("../static/app.js"));
}

#[tokio::test]
async fn unknown_static_names_are_not_found() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for url in [
        "/_static/app.css",
        "/_static/app-0000000000000000.css",
        "/_static/nope.js",
        "/_static/",
        "/_static",
        "/_static/../Cargo.toml",
        "/_static/..%2FCargo.toml",
        "/_static/%2e%2e/Cargo.toml",
        "/_static/../../src/main.rs",
    ] {
        let response = get(&app, url).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{url}");
        assert!(!response.body.contains("[package]"), "{url}");
        assert!(!response.body.contains("fn main"), "{url}");
    }
}

#[tokio::test]
async fn tree_of_subdirectory_has_breadcrumb_and_up_link() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, "/tree?ref=refs%2Fheads%2Fmain&path=src").await;
    assert_eq!(response.status, StatusCode::OK);
    let html = &response.body;
    assert!(
        html.contains("class=\"breadcrumb__current\">src<"),
        "breadcrumb current"
    );
    assert!(html.contains("class=\"tree__row tree__up\""), ".. row");
    assert!(
        html.contains("href=\"/tree?ref=refs%2Fheads%2Fmain\">..<"),
        ".. links to the parent"
    );
    assert!(
        html.contains("path=src%2Fmain.rs\""),
        "file link inside dir"
    );
    assert!(!html.contains("class=\"readme panel\""), "no README in src");

    let nested = get(&app, &tree_url("refs/heads/main", "dir with spaces")).await;
    assert_eq!(nested.status, StatusCode::OK);
    assert!(nested.body.contains("中文文件.txt"));
}

#[tokio::test]
async fn tree_at_tag_and_at_oid() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let tag = get(&app, "/tree?ref=refs%2Ftags%2Fv1.0.0").await;
    assert_eq!(tag.status, StatusCode::OK);
    assert!(
        tag.body
            .contains("class=\"ref-picker__name truncate\">v1.0.0<"),
        "tag shown as current"
    );
    assert!(
        tag.body.contains("path=assets\""),
        "tree of the tagged commit"
    );
    assert!(
        !tag.body.contains("path=page.html\""),
        "later files are absent"
    );

    let by_oid = get(&app, &format!("/tree?ref={}", repo.root_oid)).await;
    assert_eq!(by_oid.status, StatusCode::OK);
    assert!(
        by_oid
            .body
            .contains(&format!("href=\"/commit/{}\"", repo.root_oid)),
        "commit bar links the commit"
    );
}

#[tokio::test]
async fn missing_path_is_404_with_link_back() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(
        &app,
        "/tree?ref=refs%2Fheads%2Fmain&path=does%2Fnot%2Fexist",
    )
    .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let html = &response.body;
    assert!(html.contains("class=\"error-page\""), "branded error page");
    assert!(html.contains("Path not found"));
    assert!(html.contains("does/not/exist"));
    assert!(
        html.contains("href=\"/tree?ref=refs%2Fheads%2Fmain\""),
        "link back to the ref root"
    );
    assert!(
        html.contains("class=\"ref-picker\""),
        "toolbar still rendered"
    );

    // A file path is not a directory either.
    let file = get(&app, "/tree?ref=refs%2Fheads%2Fmain&path=README.md").await;
    assert_eq!(file.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invalid_ref_is_404() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for bad in [
        "nope",
        "refs%2Fheads%2Fnope",
        "main",
        "HEAD",
        "refs%2Fheads%2Fmain%5E%7Btree%7D",
        "0123456",
    ] {
        let response = get(&app, &format!("/tree?ref={bad}")).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "ref={bad}");
        assert!(response.body.contains("Ref not found"), "ref={bad}");
        assert!(response.body.contains("class=\"error-page\""), "ref={bad}");
    }
}

#[tokio::test]
async fn bad_path_is_400() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for bad in ["%2Fabs", "a%2F..%2Fb", ".", "src%2F", "a%2F%2Fb"] {
        let response = get(&app, &format!("/tree?path={bad}")).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "path={bad}");
        assert!(response.body.contains("class=\"error-page\""), "path={bad}");
        assert!(response.body.contains("Bad request"), "path={bad}");
    }
}

#[tokio::test]
async fn unknown_url_is_branded_404() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let response = get(&app, "/no/such/page").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(response.body.contains("class=\"error-page\""));
    assert!(response.body.contains("Page not found"));
    assert!(
        response.body.contains("class=\"topbar\""),
        "layout wraps the 404"
    );
}

#[tokio::test]
async fn empty_repo_shows_empty_state() {
    let repo = TempRepo::new_bare();
    let app = test_app(&repo.path);
    let response = get(&app, "/").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("class=\"empty-state\""));
    assert!(response.body.contains("No commits yet"));
    assert!(
        response.body.contains("git@example.com:test/repo.git"),
        "clone url in empty state"
    );
    assert!(
        response.body.contains("<title>repo · GitCoat</title>"),
        "bare dir name loses .git"
    );

    let tree = get(&app, "/tree").await;
    assert_eq!(tree.status, StatusCode::OK);
    assert!(tree.body.contains("class=\"empty-state\""));
    let missing = get(&app, "/tree?ref=refs%2Fheads%2Fmain").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn custom_name_without_clone_url() {
    let repo = TempRepo::rich();
    let mut config = test_config(&repo.path);
    config.name = Some("Custom Name".to_owned());
    config.clone_url = None;
    config.description = None;
    let app = test_app_with(config);
    let response = get(&app, "/").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(
        response
            .body
            .contains("<title>Custom Name · GitCoat</title>")
    );
    assert!(
        !response.body.contains("class=\"copy-wrap\""),
        "no copy button without clone url"
    );
    assert!(
        !response.body.contains("class=\"repo-desc"),
        "no description element"
    );
}

#[tokio::test]
async fn healthz_is_plain_ok() {
    let repo = TempRepo::new_bare();
    let app = test_app(&repo.path);
    let response = get(&app, "/healthz").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(
        response.content_type().starts_with("text/plain"),
        "{}",
        response.content_type()
    );
    assert_eq!(response.body, "ok\n");
}
