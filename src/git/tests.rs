//! Tests against real repositories built with the `git` CLI in temp dirs.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use super::*;
use crate::limits::{COMMITS_MAX_SKIP, TREE_MAX_ENTRIES};

/// A temporary repository driven through the `git` CLI, isolated from the
/// user's global configuration.
struct Fixture {
    _dir: tempfile::TempDir,
    home: PathBuf,
    path: PathBuf,
    commits: u32,
}

impl Fixture {
    fn new(bare: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let path = dir.path().join(if bare { "repo.git" } else { "repo" });
        let fixture = Self {
            _dir: dir,
            home,
            path,
            commits: 0,
        };
        let mut args = vec!["init", "-q", "-b", "main"];
        if bare {
            args.push("--bare");
        }
        args.push(fixture.path.to_str().unwrap());
        fixture.git_in(fixture.home.clone(), &args);
        fixture
    }

    fn git_in(&self, cwd: PathBuf, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+02:00")
            .env("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+02:00")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn git_stdin(&self, args: &[&str], stdin: &[u8]) -> String {
        use std::io::Write;
        let mut child = Command::new("git")
            .current_dir(&self.path)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("git runs");
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn git(&self, args: &[&str]) -> String {
        String::from_utf8(self.git_in(self.path.clone(), args))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn write(&self, rel: &str, content: &[u8]) {
        let file = self.path.join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }

    /// Commit everything in the work tree with a fixed, increasing date.
    fn commit(&mut self, message: &str) -> String {
        self.commits += 1;
        self.git(&["add", "-A"]);
        let date = format!("2024-01-{:02}T10:00:00+00:00", self.commits.min(28));
        self.git(&[
            "commit",
            "-q",
            "--allow-empty",
            "--date",
            &date,
            "-m",
            message,
        ]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn chmod_executable(&self, rel: &str) {
        use std::os::unix::fs::PermissionsExt;
        let file = self.path.join(rel);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn open(&self) -> Repo {
        Repo::open(&self.path).unwrap()
    }
}

fn oid(hex: &str) -> Oid {
    Oid::parse(hex, ObjectFormat::Sha1).unwrap()
}

fn path(p: &str) -> RepoPath {
    RepoPath::parse(p).unwrap()
}

#[tokio::test]
async fn opens_normal_and_bare_repositories() {
    let mut normal = Fixture::new(false);
    normal.write("a.txt", b"hello\n");
    normal.commit("first");

    let repo = normal.open();
    assert!(!repo.is_bare());
    assert_eq!(repo.object_format(), ObjectFormat::Sha1);
    assert_eq!(
        repo.git_dir(),
        normal.path.canonicalize().unwrap().join(".git")
    );
    assert!(
        Repo::open(normal.path.join(".git")).is_ok(),
        "the .git dir itself is accepted"
    );

    let bare_path = normal.path.parent().unwrap().join("clone.git");
    normal.git(&[
        "clone",
        "-q",
        "--bare",
        normal.path.to_str().unwrap(),
        bare_path.to_str().unwrap(),
    ]);
    let bare = Repo::open(&bare_path).unwrap();
    assert!(bare.is_bare());
    assert_eq!(bare.git_dir(), bare_path.canonicalize().unwrap());
    let head = bare.default_ref().await.unwrap().unwrap();
    assert_eq!(head.name, "refs/heads/main");
}

#[test]
fn rejects_non_repositories_and_subdirectories() {
    let mut fixture = Fixture::new(false);
    fixture.write("sub/dir/file.txt", b"x");
    fixture.commit("with subdir");

    assert!(matches!(
        Repo::open(fixture.path.join("sub/dir")),
        Err(OpenError::NotARepository(_))
    ));
    assert!(matches!(
        Repo::open(fixture.path.join("sub")),
        Err(OpenError::NotARepository(_))
    ));
    assert!(matches!(
        Repo::open(fixture.path.join("does-not-exist")),
        Err(OpenError::NotFound(_))
    ));
    let plain = fixture.path.parent().unwrap().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert!(matches!(
        Repo::open(&plain),
        Err(OpenError::NotARepository(_))
    ));
    assert!(matches!(
        Repo::open(fixture.path.join("a.txt")),
        Err(OpenError::NotFound(_))
    ));
}

#[tokio::test]
async fn empty_repository_has_no_default_ref() {
    let fixture = Fixture::new(true);
    let repo = fixture.open();
    assert!(repo.default_ref().await.unwrap().is_none());
    let refs = repo.list_refs().await.unwrap();
    assert!(refs.branches.is_empty() && refs.tags.is_empty());
}

#[tokio::test]
async fn default_ref_falls_back_when_head_is_unborn() {
    let mut fixture = Fixture::new(false);
    fixture.write("a", b"a");
    let first = fixture.commit("on main");
    fixture.git(&["branch", "aaa-other"]);
    fixture.git(&["symbolic-ref", "HEAD", "refs/heads/nope"]);
    let repo = fixture.open();

    let head = repo.default_ref().await.unwrap().unwrap();
    assert_eq!(head.name, "refs/heads/main");
    assert_eq!(head.short, "main");
    assert_eq!(head.kind, RefKind::Branch);
    assert_eq!(head.oid, oid(&first));

    // Without main/master, the first branch alphabetically wins, then tags.
    fixture.git(&["branch", "-m", "main", "zzz"]);
    let head = repo.default_ref().await.unwrap().unwrap();
    assert_eq!(head.name, "refs/heads/aaa-other");
    fixture.git(&["tag", "v1", &first]);
    fixture.git(&["branch", "-D", "aaa-other"]);
    fixture.git(&["branch", "-D", "zzz"]);
    let head = repo.default_ref().await.unwrap().unwrap();
    assert_eq!(
        (head.name.as_str(), head.kind),
        ("refs/tags/v1", RefKind::Tag)
    );
}

#[tokio::test]
async fn lists_and_resolves_refs() {
    let mut fixture = Fixture::new(false);
    fixture.write("a", b"a");
    let first = fixture.commit("first");
    fixture.git(&["tag", "v1.2.0"]);
    fixture.git(&["tag", "-a", "-m", "annotated", "v1.10.0"]);
    fixture.write("b", b"b");
    let second = fixture.commit("second");
    fixture.git(&["branch", "alpha", &first]);
    fixture.git(&["branch", "feature/x"]);
    // A tag pointing at a blob must be ignored.
    let blob = fixture.git(&["hash-object", "-w", "a"]);
    fixture.git(&["tag", "blob-tag", &blob]);
    let repo = fixture.open();

    let refs = repo.list_refs().await.unwrap();
    let branch_names: Vec<_> = refs.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        branch_names,
        [
            "refs/heads/main",
            "refs/heads/alpha",
            "refs/heads/feature/x"
        ]
    );
    assert_eq!(refs.branches[1].oid, oid(&first));
    let tag_names: Vec<_> = refs.tags.iter().map(|t| t.short.as_str()).collect();
    assert_eq!(tag_names, ["v1.10.0", "v1.2.0"]);
    assert_eq!(
        refs.tags[0].oid,
        oid(&first),
        "annotated tags peel to their commit"
    );

    let resolved = repo.resolve("refs/heads/feature/x").await.unwrap().unwrap();
    assert_eq!(
        (resolved.short.as_str(), resolved.kind),
        ("feature/x", RefKind::Branch)
    );
    assert_eq!(resolved.oid, oid(&second));
    let tag = repo.resolve("refs/tags/v1.10.0").await.unwrap().unwrap();
    assert_eq!((tag.kind, tag.oid.as_str()), (RefKind::Tag, first.as_str()));
    let by_oid = repo.resolve(&second).await.unwrap().unwrap();
    assert_eq!(
        (by_oid.kind, by_oid.short.as_str()),
        (RefKind::Commit, &second[..7])
    );
    let annotated_oid = fixture.git(&["rev-parse", "refs/tags/v1.10.0"]);
    assert_eq!(
        repo.resolve(&annotated_oid).await.unwrap().unwrap().oid,
        oid(&first)
    );

    for bad in [
        "main",
        "HEAD",
        "refs/heads/nope",
        "refs/tags/blob-tag",
        "refs/heads/../x",
        "abc",
        &blob,
        "",
    ] {
        assert!(
            repo.resolve(bad).await.unwrap().is_none(),
            "{bad:?} must not resolve"
        );
    }
    assert!(
        repo.resolve("refs/heads/main^{tree}")
            .await
            .unwrap()
            .is_none(),
        "no revspec syntax"
    );
}

#[tokio::test]
async fn lists_trees_with_odd_names() {
    let mut fixture = Fixture::new(false);
    fixture.write("ünï.txt", b"unicode");
    fixture.write("with space.txt", b"space!");
    fixture.write("-dash.txt", b"-");
    fixture.write("sub/inner.txt", b"inner\n");
    fixture.write("zed.txt", b"z");
    let head = fixture.commit("names");
    let repo = fixture.open();
    let commit = oid(&head);

    let listing = repo
        .ls_tree(&commit, &RepoPath::root())
        .await
        .unwrap()
        .unwrap();
    assert!(!listing.truncated);
    let names: Vec<_> = listing
        .entries
        .iter()
        .map(|e| e.name_display.as_str())
        .collect();
    assert_eq!(
        names,
        ["sub", "-dash.txt", "with space.txt", "zed.txt", "ünï.txt"]
    );
    assert_eq!(listing.entries[0].kind, EntryKind::Dir);
    assert_eq!(listing.entries[0].size, None);
    assert_eq!(listing.entries[2].size, Some(6));
    assert_eq!(listing.entries[2].mode, "100644");

    let sub = repo.ls_tree(&commit, &path("sub")).await.unwrap().unwrap();
    assert_eq!(sub.entries.len(), 1);
    assert_eq!(sub.entries[0].name, b"inner.txt");
    assert!(
        repo.ls_tree(&commit, &path("missing"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repo.ls_tree(&commit, &path("zed.txt"))
            .await
            .unwrap()
            .is_none(),
        "a file is not a dir"
    );
    assert!(
        repo.ls_tree(&commit, &path("sub/inner.txt/x"))
            .await
            .unwrap()
            .is_none()
    );

    let entry = repo
        .entry(&commit, &path("sub/inner.txt"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((entry.kind, entry.size), (EntryKind::File, Some(6)));
    let root = repo
        .entry(&commit, &RepoPath::root())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.kind, EntryKind::Dir);
    assert!(repo.entry(&commit, &path("nope")).await.unwrap().is_none());

    let (data, truncated) = repo.read_blob(&entry.oid, 3).await.unwrap().unwrap();
    assert_eq!((data.as_slice(), truncated), (&b"inn"[..], true));
    let (data, truncated) = repo.read_blob(&entry.oid, 100).await.unwrap().unwrap();
    assert_eq!((data.as_slice(), truncated), (&b"inner\n"[..], false));
    let info = repo.blob_info(&entry.oid).await.unwrap().unwrap();
    assert_eq!((info.kind, info.size), (ObjectKind::Blob, 6));
    assert!(
        repo.read_blob(&commit, 100).await.unwrap().is_none(),
        "a commit is not a blob"
    );
    assert!(
        repo.ls_tree(&oid(&"0".repeat(40)), &RepoPath::root())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn non_utf8_names_are_kept_as_bytes() {
    let fixture = Fixture::new(false);
    // The file system may refuse non-UTF-8 names, so build the tree object
    // directly: `mktree -z` takes the raw name bytes.
    let blob = fixture.git_stdin(&["hash-object", "-w", "--stdin"], b"raw");
    let mut tree_input = format!("100644 blob {blob}\t").into_bytes();
    tree_input.extend_from_slice(b"\xffname.txt\0");
    let tree = fixture.git_stdin(&["mktree", "-z"], &tree_input);
    let commit = fixture.git(&["commit-tree", &tree, "-m", "raw name"]);
    fixture.git(&["update-ref", "refs/heads/main", &commit]);
    let repo = fixture.open();

    let listing = repo
        .ls_tree(&oid(&commit), &RepoPath::root())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listing.entries.len(), 1);
    let entry = &listing.entries[0];
    assert_eq!(entry.name, b"\xffname.txt");
    assert!(!entry.is_valid_utf8());
    assert_eq!(entry.name_display, "\u{fffd}name.txt");

    // The raw bytes look the file up again; the lossy name does not.
    let raw = RepoPath::from_bytes(entry.name.clone()).unwrap();
    assert!(repo.entry(&oid(&commit), &raw).await.unwrap().is_some());
    let lossy = RepoPath::parse(&entry.name_display).unwrap();
    assert!(repo.entry(&oid(&commit), &lossy).await.unwrap().is_none());
}

#[tokio::test]
async fn finds_readmes_by_priority() {
    let mut fixture = Fixture::new(false);
    fixture.write("readme", b"plain");
    fixture.write("Readme.markdown", b"markdown");
    fixture.write("docs/README.md", b"docs md");
    let head = fixture.commit("readmes");
    let repo = fixture.open();
    let commit = oid(&head);

    let readme = repo
        .readme(&commit, &RepoPath::root())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (readme.entry.name_display.as_str(), readme.data.as_slice()),
        ("Readme.markdown", &b"markdown"[..])
    );
    assert!(!readme.truncated);
    let docs = repo.readme(&commit, &path("docs")).await.unwrap().unwrap();
    assert_eq!(docs.data, b"docs md");
    fixture.write("sub/other.txt", b"");
    let head = fixture.commit("no readme in sub");
    assert!(
        repo.readme(&oid(&head), &path("sub"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repo.readme(&oid(&head), &path("missing"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn log_pages_with_has_more() {
    let mut fixture = Fixture::new(false);
    let mut heads = Vec::new();
    for i in 0..7 {
        fixture.write("counter", format!("{i}\n").as_bytes());
        heads.push(fixture.commit(&format!("commit {i}")));
    }
    let repo = fixture.open();
    let head = oid(heads.last().unwrap());

    let (page, has_more) = repo.log(&head, 0, 3).await.unwrap();
    assert!(has_more);
    let titles: Vec<_> = page.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["commit 6", "commit 5", "commit 4"]);
    assert_eq!(page[0].oid, head);
    assert_eq!(page[0].parents, vec![oid(&heads[5])]);
    assert_eq!(page[0].author_name, "Test");
    assert_eq!(page[0].author_email, "test@example.com");
    assert_eq!(page[0].author_time.tz_offset_minutes, 0);
    assert_eq!(
        page[0].author_time.unix, 1_704_621_600,
        "2024-01-07T10:00:00Z"
    );

    let (page, has_more) = repo.log(&head, 3, 3).await.unwrap();
    assert!(has_more);
    assert_eq!(
        page.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
        ["commit 3", "commit 2", "commit 1"]
    );
    let (page, has_more) = repo.log(&head, 6, 3).await.unwrap();
    assert!(!has_more);
    assert_eq!(page.len(), 1);
    assert!(page[0].parents.is_empty());
    let (page, has_more) = repo.log(&head, 7, 3).await.unwrap();
    assert!(page.is_empty() && !has_more);
    assert!(matches!(
        repo.log(&head, COMMITS_MAX_SKIP + 1, 3).await,
        Err(GitError::NotFound(_))
    ));
    assert!(matches!(
        repo.log(&oid(&"1".repeat(40)), 0, 3).await,
        Err(GitError::NotFound(_))
    ));

    let detail = repo.commit(&head).await.unwrap().unwrap();
    assert_eq!(detail.title, "commit 6");
    assert_eq!(detail.committer_name, "Test");
    assert_eq!(detail.body, "");
    assert!(repo.commit(&oid(&"2".repeat(40))).await.unwrap().is_none());
}

#[tokio::test]
async fn commit_body_and_time_zone() {
    let fixture = Fixture::new(false);
    fixture.write("f", b"f");
    fixture.git(&["add", "-A"]);
    fixture.git(&[
        "commit",
        "-q",
        "--date",
        "2024-01-02T03:04:05+02:00",
        "-m",
        "Title line\n\nBody line one\nline two\n",
    ]);
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let repo = fixture.open();
    let detail = repo.commit(&oid(&head)).await.unwrap().unwrap();
    assert_eq!(detail.title, "Title line");
    assert_eq!(detail.body, "Body line one\nline two");
    assert_eq!(detail.author_time.tz_offset_minutes, 120);
    assert_eq!(detail.author_time.unix, 1_704_157_445);
}

#[tokio::test]
async fn diffs_root_merge_rename_mode_and_binary() {
    let mut fixture = Fixture::new(false);
    fixture.write("a.txt", b"one\ntwo\nthree\n");
    fixture.write("bin.dat", b"\x00\x01\x02binary");
    let root = fixture.commit("root");
    let repo = fixture.open();

    let diff = repo.diff(&oid(&root)).await.unwrap();
    assert!(diff.compared_against.is_none());
    assert!(!diff.truncated);
    assert_eq!(diff.files.len(), 2);
    let a = diff
        .files
        .iter()
        .find(|f| f.path().display() == "a.txt")
        .unwrap();
    assert_eq!(a.status, DiffStatus::Added);
    assert!(a.old_path.is_none());
    assert_eq!(a.new_mode.as_deref(), Some("100644"));
    assert_eq!((a.additions, a.deletions), (Some(3), Some(0)));
    assert_eq!(a.hunks.len(), 1);
    assert_eq!(a.hunks[0].lines[2].text, "three");
    let bin = diff
        .files
        .iter()
        .find(|f| f.path().display() == "bin.dat")
        .unwrap();
    assert!(bin.is_binary);
    assert!(bin.hunks.is_empty());
    assert_eq!((bin.additions, bin.deletions), (None, None));
    assert_eq!((diff.additions, diff.deletions), (3, 0));

    // Rename + modification + mode change + deletion on a branch.
    fixture.git(&["checkout", "-q", "-b", "topic"]);
    std::fs::rename(fixture.path.join("a.txt"), fixture.path.join("renamed.txt")).unwrap();
    fixture.write("renamed.txt", b"one\ntwo\nthree\nfour\n");
    fixture.write("script.sh", b"#!/bin/sh\n");
    std::fs::remove_file(fixture.path.join("bin.dat")).unwrap();
    let topic = fixture.commit("topic work");
    fixture.chmod_executable("script.sh");
    let chmod = fixture.commit("make executable");

    let diff = repo.diff(&oid(&chmod)).await.unwrap();
    assert_eq!(diff.compared_against, Some(oid(&topic)));
    assert_eq!(diff.files.len(), 1);
    let script = &diff.files[0];
    assert_eq!(script.status, DiffStatus::ModeChanged);
    assert_eq!(
        (script.old_mode.as_deref(), script.new_mode.as_deref()),
        (Some("100644"), Some("100755"))
    );
    assert!(script.hunks.is_empty());

    let diff = repo.diff(&oid(&topic)).await.unwrap();
    assert_eq!(diff.compared_against, Some(oid(&root)));
    let renamed = diff
        .files
        .iter()
        .find(|f| f.path().display() == "renamed.txt")
        .unwrap();
    assert_eq!(
        renamed.status,
        DiffStatus::Renamed {
            from: path("a.txt")
        }
    );
    assert_eq!(renamed.old_path.as_ref().unwrap().display(), "a.txt");
    assert_eq!((renamed.additions, renamed.deletions), (Some(1), Some(0)));
    assert_eq!(renamed.hunks[0].header, "@@ -1,3 +1,4 @@");
    let deleted = diff
        .files
        .iter()
        .find(|f| f.path().display() == "bin.dat")
        .unwrap();
    assert_eq!(deleted.status, DiffStatus::Deleted);
    assert!(deleted.is_binary && deleted.new_path.is_none());
    let added = diff
        .files
        .iter()
        .find(|f| f.path().display() == "script.sh")
        .unwrap();
    assert_eq!(added.status, DiffStatus::Added);

    // Merge: compared against the first parent only.
    fixture.git(&["checkout", "-q", "main"]);
    fixture.write("main-only.txt", b"main\n");
    let main2 = fixture.commit("main work");
    fixture.git(&["merge", "-q", "--no-ff", "--no-edit", "topic"]);
    let merge = fixture.git(&["rev-parse", "HEAD"]);
    let detail = repo.commit(&oid(&merge)).await.unwrap().unwrap();
    assert_eq!(detail.parents, vec![oid(&main2), oid(&chmod)]);
    let diff = repo.diff(&oid(&merge)).await.unwrap();
    assert_eq!(diff.compared_against, Some(oid(&main2)));
    let mut changed: Vec<_> = diff.files.iter().map(|f| f.path().display()).collect();
    changed.sort();
    assert_eq!(changed, ["bin.dat", "renamed.txt", "script.sh"]);
    let renamed = diff
        .files
        .iter()
        .find(|f| f.path().display() == "renamed.txt")
        .unwrap();
    assert_eq!(
        renamed.status,
        DiffStatus::Renamed {
            from: path("a.txt")
        }
    );
    assert!(
        diff.files
            .iter()
            .all(|f| f.path().display() != "main-only.txt")
    );

    // Modification of an existing file.
    fixture.write("renamed.txt", b"one\nTWO\nthree\nfour\n");
    let modified = fixture.commit("edit");
    let diff = repo.diff(&oid(&modified)).await.unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].status, DiffStatus::Modified);
    assert_eq!(
        (diff.files[0].additions, diff.files[0].deletions),
        (Some(1), Some(1))
    );
    let lines = &diff.files[0].hunks[0].lines;
    assert_eq!(
        (lines[1].kind, lines[1].old_no, lines[1].text.as_str()),
        (LineKind::Del, Some(2), "two")
    );
    assert_eq!(
        (lines[2].kind, lines[2].new_no, lines[2].text.as_str()),
        (LineKind::Add, Some(2), "TWO")
    );

    assert!(matches!(
        repo.diff(&oid(&"3".repeat(40))).await,
        Err(GitError::NotFound(_))
    ));
}

#[tokio::test]
async fn tree_listing_is_capped() {
    let mut fixture = Fixture::new(false);
    for i in 0..(TREE_MAX_ENTRIES + 5) {
        fixture.write(&format!("f{i:05}"), b"");
    }
    let head = fixture.commit("many");
    let repo = fixture.open();
    let listing = repo
        .ls_tree(&oid(&head), &RepoPath::root())
        .await
        .unwrap()
        .unwrap();
    assert!(listing.truncated);
    assert_eq!(listing.entries.len(), TREE_MAX_ENTRIES);
}

#[tokio::test]
async fn sees_refs_pushed_after_open_and_packed_objects() {
    let mut fixture = Fixture::new(false);
    fixture.write("a", b"a");
    let first = fixture.commit("first");
    let repo = fixture.open();
    assert_eq!(repo.list_refs().await.unwrap().branches.len(), 1);

    fixture.git(&["branch", "later"]);
    fixture.write("b", b"b");
    let second = fixture.commit("second");
    assert_eq!(repo.list_refs().await.unwrap().branches.len(), 2);
    assert_eq!(repo.default_ref().await.unwrap().unwrap().oid, oid(&second));

    fixture.git(&["pack-refs", "--all"]);
    fixture.git(&["gc", "-q", "--prune=now"]);
    assert_eq!(repo.list_refs().await.unwrap().branches.len(), 2);
    assert!(repo.commit(&oid(&first)).await.unwrap().is_some());
    assert!(
        repo.ls_tree(&oid(&second), &RepoPath::root())
            .await
            .unwrap()
            .is_some()
    );
}

#[test]
fn fixture_paths_are_absolute() {
    let fixture = Fixture::new(true);
    assert!(Path::new(&fixture.path).is_absolute());
}
