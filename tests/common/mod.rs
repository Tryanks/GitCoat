//! Shared helpers for the integration tests: repository fixtures, an
//! in-process router with a test asset bundle, and a tiny HTTP client.

#![allow(dead_code)]

pub mod fixtures;

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use gitcoat::{
    app::{
        AppState,
        layout::{APP_CSS, APP_JS},
        router,
    },
    config::Config,
    git::Repo,
};
use http::{HeaderMap, StatusCode};
use http_body_util::BodyExt;
use topcoat::{
    asset::{AssetBundle, MANIFEST_NAME, MANIFEST_VERSION, Manifest, ManifestEntry},
    router::{Body, Router},
};

/// The asset bundle used by every test router: a manifest describing the
/// real `static/` files under the asset ids this build declared.
fn test_assets() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().expect("current exe");
        let mut hasher = DefaultHasher::new();
        exe.hash(&mut hasher);
        let dir =
            std::env::temp_dir().join(format!("gitcoat-test-assets-{:016x}", hasher.finish()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create asset dir");

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let entries = [
            (APP_CSS, "static/app.css", "app-test.css", "text/css"),
            (APP_JS, "static/app.js", "app-test.js", "text/javascript"),
        ];
        let mut manifest = Manifest {
            version: MANIFEST_VERSION,
            assets: Vec::new(),
        };
        for (asset, source, file, content_type) in entries {
            std::fs::copy(root.join(source), dir.join(file)).expect("copy static file");
            manifest.assets.push(ManifestEntry {
                id: asset.id(),
                file: file.to_owned(),
                hash: "test".to_owned(),
                content_type: content_type.to_owned(),
            });
        }
        manifest
            .save(dir.join(MANIFEST_NAME))
            .expect("write manifest");
        dir
    })
}

/// Configuration a test app runs with.
pub fn test_config(repo_path: &Path) -> Config {
    Config {
        repo: repo_path.to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        name: None,
        description: Some("A test repository".to_owned()),
        clone_url: Some("git@example.com:test/repo.git".to_owned()),
        assets_dir: None,
    }
}

/// Build the application router for the repository at `repo_path`.
pub fn test_app(repo_path: &Path) -> Router {
    test_app_with(test_config(repo_path))
}

/// Build the application router with an explicit configuration.
pub fn test_app_with(config: Config) -> Router {
    let repo = Repo::open(&config.repo).expect("open fixture repository");
    let assets = AssetBundle::load_dir(test_assets()).expect("load test asset bundle");
    router(
        AppState {
            config,
            repo: Arc::new(repo),
        },
        assets,
    )
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
    let start = html.find("<link rel=\"stylesheet\" href=\"")?;
    let rest = &html[start + "<link rel=\"stylesheet\" href=\"".len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}
