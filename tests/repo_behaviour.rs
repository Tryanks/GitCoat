//! Repository-level behaviour of the tree pages: bare vs normal repositories,
//! ref handling, freshness after external changes, read-only guarantees and
//! hostile inputs. Only `/`, `/tree`, `/healthz` and assets are exercised.

mod common;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::SystemTime,
};

use common::{fixtures::TempRepo, get, test_app};
use http::StatusCode;

/// A shared, never-mutated rich normal repository.
fn rich() -> &'static TempRepo {
    static RICH: OnceLock<TempRepo> = OnceLock::new();
    RICH.get_or_init(TempRepo::rich)
}

/// A shared, never-mutated rich bare repository.
fn rich_bare() -> &'static TempRepo {
    static BARE: OnceLock<TempRepo> = OnceLock::new();
    BARE.get_or_init(TempRepo::rich_bare)
}

const MAIN: &str = "refs%2Fheads%2Fmain";
const README_TITLE: &str = "<h1 id=\"gitcoat-fixture\">GitCoat fixture</h1>";

fn commit_link(oid: &str) -> String {
    format!("href=\"/commit/{oid}\"")
}

// ---------------------------------------------------------------------------
// (a) bare repositories
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bare_repo_home_lists_files_and_readme() {
    let repo = rich_bare();
    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    let html = &home.body;
    assert!(
        html.contains("<title>repo · GitCoat</title>"),
        "bare name loses .git"
    );
    assert!(
        html.contains(&format!("href=\"/tree?ref={MAIN}&amp;path=src\"")),
        "src dir"
    );
    assert!(
        html.contains(&format!("href=\"/blob?ref={MAIN}&amp;path=README.md\"")),
        "README"
    );
    assert!(html.contains("class=\"readme panel\""), "README panel");
    assert!(html.contains(README_TITLE), "README content");
    assert!(
        html.contains(&commit_link(&repo.mode_change_oid)),
        "latest commit on main"
    );

    let sub = get(&app, &format!("/tree?ref={MAIN}&path=docs")).await;
    assert_eq!(sub.status, StatusCode::OK);
    assert!(
        sub.body.contains("path=docs%2FGUIDE.md\""),
        "renamed guide on main"
    );
}

// ---------------------------------------------------------------------------
// (b) only committed content is shown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn uncommitted_worktree_changes_are_invisible() {
    let repo = TempRepo::rich().with_uncommitted_changes();
    assert!(
        repo.path.join("UNCOMMITTED.txt").exists(),
        "fixture wrote the file"
    );
    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    let html = &home.body;
    assert!(html.contains(README_TITLE), "committed README text");
    assert!(
        !html.contains("UNCOMMITTED CHANGE"),
        "modified README text must not leak"
    );
    assert!(
        !html.contains("UNCOMMITTED.txt"),
        "untracked file must not be listed"
    );

    let tree = get(&app, "/tree").await;
    assert_eq!(tree.status, StatusCode::OK);
    assert!(!tree.body.contains("UNCOMMITTED"));
}

// ---------------------------------------------------------------------------
// (c) switching refs keeps the path; missing path links back to the same ref
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ref_switch_keeps_path_and_missing_path_links_back_to_ref() {
    let repo = rich();
    let app = test_app(&repo.path);
    let feature = "refs%2Fheads%2Ffeature%2Fwith-slash";
    let page = get(&app, &format!("/tree?ref={feature}&path=src")).await;
    assert_eq!(page.status, StatusCode::OK);
    let html = &page.body;
    assert!(
        html.contains("class=\"breadcrumb__current\">src<"),
        "breadcrumb"
    );
    assert!(
        html.contains(&format!(
            "href=\"/blob?ref={feature}&amp;path=src%2Fmain.rs\""
        )),
        "file links stay on the feature branch"
    );
    assert!(
        html.contains(&format!("href=\"/tree?ref={feature}\">..<")),
        ".. stays on the feature branch"
    );
    assert!(
        html.contains(&format!(
            "href=\"/tree?ref={MAIN}&amp;path=src\" aria-selected=\"false\""
        )),
        "picker items keep the current path"
    );
    assert!(
        html.contains(&format!(
            "href=\"/tree?ref={feature}&amp;path=src\" aria-selected=\"true\""
        )),
        "feature branch is the selected picker item"
    );

    // `assets/` was added after the commit `dup` points at.
    let dup = "refs%2Fheads%2Fdup";
    let missing = get(&app, &format!("/tree?ref={dup}&path=assets")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let html = &missing.body;
    assert!(html.contains("Path not found"));
    let actions_at = html.find("error-page__actions").expect("actions block");
    let actions = &html[actions_at..];
    let end = actions.find("</section>").unwrap_or(actions.len());
    let actions = &actions[..end];
    assert!(
        actions.contains(&format!("href=\"/tree?ref={dup}\"")),
        "link back goes to the dup branch root: {actions}"
    );
    assert!(
        !actions.contains(MAIN),
        "link back must not switch to main: {actions}"
    );
}

// ---------------------------------------------------------------------------
// (d) branch vs tag with the same short name; annotated tags
// ---------------------------------------------------------------------------

#[tokio::test]
async fn same_named_branch_and_tag_are_distinct_and_annotated_tags_browse() {
    let repo = rich();
    let app = test_app(&repo.path);

    let branch = get(&app, "/tree?ref=refs%2Fheads%2Fdup").await;
    assert_eq!(branch.status, StatusCode::OK);
    assert!(branch.body.contains(&commit_link(&repo.dup_branch_oid)));
    assert!(
        !branch.body.contains("path=assets\""),
        "root commit has no assets/"
    );

    let tag = get(&app, "/tree?ref=refs%2Ftags%2Fdup").await;
    assert_eq!(tag.status, StatusCode::OK);
    assert!(tag.body.contains(&commit_link(&repo.dup_tag_oid)));
    assert!(
        tag.body.contains("path=assets\""),
        "tagged commit has assets/"
    );
    assert_ne!(repo.dup_branch_oid, repo.dup_tag_oid);
    assert!(
        tag.body
            .contains("href=\"/tree?ref=refs%2Ftags%2Fdup\" aria-selected=\"true\""),
        "tag selected in picker"
    );
    assert!(
        tag.body
            .contains("href=\"/tree?ref=refs%2Fheads%2Fdup\" aria-selected=\"false\""),
        "branch not selected in picker"
    );

    let annotated = get(&app, "/tree?ref=refs%2Ftags%2Fv1.0.0").await;
    assert_eq!(annotated.status, StatusCode::OK);
    assert!(
        annotated.body.contains(&commit_link(&repo.v1_oid)),
        "annotated tag peels to its commit"
    );
    assert!(
        !annotated.body.contains(&repo.v1_tag_oid),
        "the tag object id itself is not shown as the commit"
    );
    let inner = get(&app, "/tree?ref=refs%2Ftags%2Fv1.0.0&path=assets").await;
    assert_eq!(inner.status, StatusCode::OK);
    assert!(inner.body.contains("path=assets%2Flogo.png\""));
}

// ---------------------------------------------------------------------------
// (e) empty repository and a HEAD that points at a missing branch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_bare_repo_and_dangling_head_fall_back() {
    let empty = TempRepo::empty_bare();
    let app = test_app(&empty.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.contains("class=\"empty-state\""));
    assert!(home.body.contains("No commits yet"));

    let mut repo = TempRepo::rich();
    repo.set_head("refs/heads/gone");
    assert_eq!(repo.head_ref(), "refs/heads/gone");
    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    let html = &home.body;
    assert!(!html.contains("class=\"empty-state\""), "not an empty repo");
    assert!(html.contains(README_TITLE), "main's README");
    assert!(
        html.contains(&format!("href=\"/tree?ref={MAIN}\" aria-selected=\"true\"")),
        "main is marked as the current ref"
    );
    assert!(
        !html.contains("refs%2Fheads%2Fgone"),
        "the dangling branch is not listed"
    );
    assert!(
        html.contains(&commit_link(&repo.mode_change_oid)),
        "main's tip"
    );
}

// ---------------------------------------------------------------------------
// (f) external pushes are visible without restarting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_push_shows_on_next_request() {
    let mut repo = TempRepo::rich_bare();
    let app = test_app(&repo.path);
    let before = get(&app, "/").await;
    assert_eq!(before.status, StatusCode::OK);
    assert!(!before.body.contains("path=pushed-"));

    let pushed = repo.push_new_commit_from_clone("External push");
    let after = get(&app, "/").await;
    assert_eq!(after.status, StatusCode::OK);
    assert!(
        after.body.contains("path=pushed-"),
        "pushed file listed without rebuilding the router"
    );
    assert!(
        after.body.contains(&commit_link(&pushed)),
        "commit bar shows the pushed commit"
    );
    assert!(after.body.contains("External push"));
}

// ---------------------------------------------------------------------------
// (g) pack-refs + gc after the repository was opened
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pages_survive_pack_refs_and_gc() {
    let mut repo = TempRepo::rich();
    let app = test_app(&repo.path);
    assert_eq!(get(&app, "/").await.status, StatusCode::OK);

    repo.pack_refs_and_gc();
    assert!(
        repo.git_dir().join("packed-refs").exists(),
        "refs were packed"
    );

    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    let html = &home.body;
    assert!(html.contains(README_TITLE));
    for reference in [
        "refs%2Fheads%2Fmain",
        "refs%2Fheads%2Ffeature%2Fwith-slash",
        "refs%2Fheads%2Fdup",
        "refs%2Ftags%2Fv1.0.0",
        "refs%2Ftags%2Fv0.9",
        "refs%2Ftags%2Fdup",
    ] {
        assert!(
            html.contains(&format!("href=\"/tree?ref={reference}\"")),
            "{reference} listed"
        );
    }
    let tag = get(&app, "/tree?ref=refs%2Ftags%2Fv1.0.0&path=src").await;
    assert_eq!(tag.status, StatusCode::OK);
    let by_oid = get(&app, &format!("/tree?ref={}", repo.root_oid)).await;
    assert_eq!(by_oid.status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// (h) browsing never writes into the repository
// ---------------------------------------------------------------------------

type Snapshot = BTreeMap<PathBuf, (SystemTime, u64)>;

fn snapshot(dir: &Path) -> Snapshot {
    fn walk(root: &Path, dir: &Path, out: &mut Snapshot) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).expect("metadata");
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            out.insert(rel, (meta.modified().expect("mtime"), meta.len()));
            if meta.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Snapshot::new();
    walk(dir, dir, &mut out);
    out
}

async fn browse_everything(repo: &TempRepo) {
    let app = test_app(&repo.path);
    let urls = [
        "/".to_owned(),
        "/tree".to_owned(),
        format!("/tree?ref={MAIN}&path=src"),
        format!("/tree?ref={MAIN}&path=docs"),
        format!("/tree?ref={MAIN}&path=dir+with+spaces"),
        "/tree?ref=refs%2Fheads%2Ffeature%2Fwith-slash".to_owned(),
        "/tree?ref=refs%2Ftags%2Fv1.0.0&path=assets".to_owned(),
        "/tree?ref=refs%2Ftags%2Fdup".to_owned(),
        format!("/tree?ref={}", repo.root_oid),
        format!("/tree?ref={MAIN}&path=does%2Fnot%2Fexist"),
        "/tree?ref=refs%2Fheads%2Fnope".to_owned(),
        "/tree?path=..%2Fx".to_owned(),
        "/healthz".to_owned(),
    ];
    for url in urls {
        let response = get(&app, &url).await;
        assert!(
            response.status.is_success() || response.status.is_client_error(),
            "{url}: {}",
            response.status
        );
    }
}

fn assert_untouched(before: &Snapshot, after: &Snapshot) {
    let created: Vec<_> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    let removed: Vec<_> = before.keys().filter(|k| !after.contains_key(*k)).collect();
    let changed: Vec<_> = before
        .iter()
        .filter(|(k, v)| after.get(*k).is_some_and(|a| a != *v))
        .map(|(k, _)| k)
        .collect();
    assert!(
        created.is_empty(),
        "files created while browsing: {created:?}"
    );
    assert!(
        removed.is_empty(),
        "files removed while browsing: {removed:?}"
    );
    assert!(
        changed.is_empty(),
        "files modified while browsing: {changed:?}"
    );
    for path in after.keys() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let suspicious = name.ends_with(".lock") || name == "ORIG_HEAD" || name == "index";
        assert!(
            !suspicious || before.contains_key(path),
            "{} appeared while browsing",
            path.display()
        );
    }
}

#[tokio::test]
async fn browsing_does_not_write_into_a_bare_repo() {
    let repo = rich_bare();
    let git_dir = repo.git_dir();
    assert_eq!(git_dir, repo.path);
    let before = snapshot(&git_dir);
    browse_everything(repo).await;
    let after = snapshot(&git_dir);
    assert_untouched(&before, &after);
    assert!(
        !after.keys().any(|p| p.ends_with("index")),
        "a bare repo never gains an index"
    );
}

#[tokio::test]
async fn browsing_does_not_write_into_a_normal_repo() {
    let repo = rich();
    let git_dir = repo.git_dir();
    assert!(git_dir.ends_with(".git"));
    let before = snapshot(&git_dir);
    let before_worktree = snapshot(&repo.path);
    browse_everything(repo).await;
    assert_untouched(&before, &snapshot(&git_dir));
    assert_untouched(&before_worktree, &snapshot(&repo.path));
}

// ---------------------------------------------------------------------------
// (i) a read-only repository is enough
// ---------------------------------------------------------------------------

/// Removes write permission from a tree and restores it on drop so the
/// temporary directory can be deleted.
struct ReadOnlyGuard {
    saved: Vec<(PathBuf, fs::Permissions)>,
}

impl ReadOnlyGuard {
    fn new(root: &Path) -> Self {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).expect("read dir") {
                let path = entry.expect("entry").path();
                if fs::symlink_metadata(&path).expect("meta").is_dir() {
                    walk(&path, out);
                }
                out.push(path);
            }
        }
        let mut paths = Vec::new();
        walk(root, &mut paths);
        paths.push(root.to_path_buf());
        let mut saved = Vec::new();
        for path in paths {
            let meta = fs::symlink_metadata(&path).expect("meta");
            if meta.file_type().is_symlink() {
                continue;
            }
            let original = meta.permissions();
            let mut readonly = original.clone();
            readonly.set_readonly(true);
            fs::set_permissions(&path, readonly).expect("chmod a-w");
            saved.push((path, original));
        }
        Self { saved }
    }
}

impl Drop for ReadOnlyGuard {
    fn drop(&mut self) {
        // Directories first (deepest last in `saved`), so restore in reverse.
        for (path, perms) in self.saved.iter().rev() {
            let _ = fs::set_permissions(path, perms.clone());
        }
    }
}

#[tokio::test]
async fn read_only_repository_is_browsable() {
    let repo = TempRepo::rich_bare();
    let guard = ReadOnlyGuard::new(&repo.path);
    assert!(
        fs::metadata(&repo.path).unwrap().permissions().readonly(),
        "fixture is read-only"
    );
    assert!(
        fs::write(repo.path.join("probe"), b"x").is_err(),
        "the directory really refuses writes"
    );

    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.contains(README_TITLE));
    let tag = get(&app, "/tree?ref=refs%2Ftags%2Fv1.0.0&path=src").await;
    assert_eq!(tag.status, StatusCode::OK);
    assert!(tag.body.contains("path=src%2Fmain.rs\""));
    drop(guard);
}

// ---------------------------------------------------------------------------
// (j) symlinks are shown, never followed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn symlinks_are_badged_and_never_expanded() {
    let mut repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    let html = &home.body;
    let row_at = html.find("path=link\"").expect("symlink entry is linked");
    let row = &html[row_at..html[row_at..].find("</tr>").map(|i| row_at + i).unwrap()];
    assert!(
        row.contains("class=\"badge\">symlink<"),
        "symlink badge: {row}"
    );
    assert!(
        html.contains(&format!("href=\"/blob?ref={MAIN}&amp;path=link\"")),
        "symlink links to its own blob, not the target"
    );
    assert!(
        !html.contains("//! GitCoat fixture program."),
        "target content is not inlined"
    );
    // The symlink is a leaf: asking for it as a directory is a 404, not a
    // listing of src/.
    let as_dir = get(&app, &format!("/tree?ref={MAIN}&path=link")).await;
    assert_eq!(as_dir.status, StatusCode::NOT_FOUND);
    assert!(!as_dir.body.contains("path=link%2Fmain.rs"));

    // A symlink escaping the repository is displayed like any other.
    repo.commit_symlink("escape", "/etc/hosts", "Add symlink to /etc/hosts");
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    let html = &home.body;
    assert!(html.contains("path=escape\""), "escape entry listed");
    let row_at = html.find("path=escape\"").unwrap();
    let row = &html[row_at..html[row_at..].find("</tr>").map(|i| row_at + i).unwrap()];
    assert!(
        row.contains("class=\"badge\">symlink<"),
        "escape is a symlink: {row}"
    );

    if let Ok(hosts) = fs::read_to_string("/etc/hosts") {
        let lines: Vec<&str> = hosts
            .lines()
            .map(str::trim_end)
            .filter(|l| l.len() >= 12 && !l.starts_with('#'))
            .collect();
        assert!(
            !lines.is_empty(),
            "/etc/hosts has content to compare against"
        );
        for line in lines {
            assert!(
                !html.contains(line),
                "/etc/hosts line leaked into the page: {line:?}"
            );
        }
    }
    let as_dir = get(&app, &format!("/tree?ref={MAIN}&path=escape")).await;
    assert_eq!(as_dir.status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// (k) hostile file names
// ---------------------------------------------------------------------------

#[tokio::test]
async fn newline_and_invalid_utf8_names_do_not_break_the_tree() {
    let mut repo = TempRepo::rich();
    let has_newline = repo.add_newline_name_file();
    let has_invalid = repo.add_invalid_utf8_name_file();
    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    let html = &home.body;
    assert!(html.contains(README_TITLE), "page still complete");

    if has_newline {
        assert!(
            html.contains(&format!(
                "href=\"/blob?ref={MAIN}&amp;path=new%0Aline.txt\""
            )),
            "newline name is linked with the newline encoded"
        );
    }
    if has_invalid {
        let display = "bad\u{FFFD}.txt";
        assert!(
            html.contains(&format!("<span>{display}</span>")),
            "lossy name shown"
        );
        let row_at = html.find(&format!("<span>{display}</span>")).unwrap();
        let row = &html[row_at..html[row_at..].find("</tr>").map(|i| row_at + i).unwrap()];
        assert!(
            row.contains("class=\"badge\">unsupported name<"),
            "unsupported name badge: {row}"
        );
        assert!(
            !row.contains("<a "),
            "no link for an unrepresentable name: {row}"
        );
        assert!(
            !html.contains("path=bad%EF%BF%BD.txt"),
            "no lossy path link anywhere"
        );
        assert!(
            !html.contains("path=bad%FF.txt"),
            "no raw-byte path link either"
        );
    }
    assert!(
        has_newline || has_invalid,
        "git refused both odd names; nothing was tested"
    );
}

// ---------------------------------------------------------------------------
// (l) path traversal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn path_traversal_never_yields_file_content() {
    let repo = rich();
    let app = test_app(&repo.path);
    let git_config = fs::read_to_string(repo.git_dir().join("config")).expect("git config");
    assert!(git_config.contains("[core]"));

    let bad_request = [
        "..",
        "..%2F",
        "a%2F..%2F..%2Fb",
        "%2Fetc%2Fpasswd",
        "..%2F..%2F..%2Fetc%2Fpasswd",
        "src%2F..%2F.git%2Fconfig",
    ];
    for path in bad_request {
        let response = get(&app, &format!("/tree?path={path}")).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "path={path}");
        assert!(!response.body.contains("[core]"), "path={path}");
        assert!(!response.body.contains("root:"), "path={path}");
    }

    // `.git` is not part of any tree; a normal repo's metadata is unreachable.
    for path in [".git", ".git%2Fconfig", ".git%2Frefs", ".git%2FHEAD"] {
        let response = get(&app, &format!("/tree?path={path}")).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "path={path}");
        assert!(!response.body.contains("[core]"), "path={path}");
        assert!(
            !response.body.contains("repositoryformatversion"),
            "path={path}"
        );
    }

    // Static asset handler.
    for url in [
        "/_topcoat/assets/../../Cargo.toml",
        "/_topcoat/assets/..%2F..%2FCargo.toml",
        "/_topcoat/assets/%2e%2e/%2e%2e/Cargo.toml",
        "/_topcoat/assets/../../../../etc/passwd",
    ] {
        let response = get(&app, url).await;
        let leaked = response.status == StatusCode::OK
            && (response.body.contains("[package]") || response.body.contains("root:"));
        assert!(
            !leaked,
            "{url} served a file outside the bundle: {}",
            response.status
        );
        assert!(!response.body.contains("name = \"gitcoat\""), "{url}");
    }
}
