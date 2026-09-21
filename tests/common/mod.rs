//! Shared helpers for the integration tests: repository fixtures, an
//! in-process router and a tiny HTTP client.

#![allow(dead_code)]

pub mod fixtures;

use std::{path::Path, sync::Arc};

use gitcoat::{
    app::{AppState, router},
    config::Config,
    git::Repo,
};
use http::{HeaderMap, StatusCode};
use http_body_util::BodyExt;
use topcoat::router::{Body, Router};

/// Configuration a test app runs with.
pub fn test_config(repo_path: &Path) -> Config {
    Config {
        repo: repo_path.to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        name: None,
        description: Some("A test repository".to_owned()),
        clone_url: Some("git@example.com:test/repo.git".to_owned()),
    }
}

/// Build the application router for the repository at `repo_path`.
pub fn test_app(repo_path: &Path) -> Router {
    test_app_with(test_config(repo_path))
}

/// Build the application router with an explicit configuration.
pub fn test_app_with(config: Config) -> Router {
    let repo = Repo::open(&config.repo).expect("open fixture repository");
    router(AppState {
        config,
        repo: Arc::new(repo),
    })
}

/// A collected response.
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
    pub bytes: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }

    pub fn content_type(&self) -> &str {
        self.header("content-type")
    }
}

/// `GET path` through the router, without a listener.
pub async fn get(router: &Router, path: &str) -> Response {
    get_with_headers(router, path, &[]).await
}

/// `GET path` with extra request headers (e.g. `Accept-Language`, `Cookie`).
pub async fn get_with_headers(router: &Router, path: &str, headers: &[(&str, &str)]) -> Response {
    let mut request = http::Request::builder()
        .method(http::Method::GET)
        .uri(path)
        .header("host", "localhost");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request.body(Body::empty()).expect("valid request");
    let response = router.handle(request).await;
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.expect("read body").to_bytes().to_vec();
    Response {
        status: parts.status,
        headers: parts.headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        bytes,
    }
}

/// The href of the stylesheet `<link>` in an HTML page.
pub fn stylesheet_href(html: &str) -> Option<String> {
    attribute_after(html, "<link rel=\"stylesheet\" href=\"")
}

/// The src of the deferred `<script>` in an HTML page.
pub fn script_src(html: &str) -> Option<String> {
    attribute_after(html, "<script src=\"")
}

fn attribute_after(html: &str, prefix: &str) -> Option<String> {
    let start = html.find(prefix)?;
    let rest = &html[start + prefix.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}
