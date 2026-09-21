//! Git repository fixtures for integration tests.
//!
//! Every repository is built with the `git` CLI through `std::process::Command`
//! (args arrays, never a shell) inside a private temporary directory, with a
//! fully deterministic environment: no global/system config, fixed author and
//! committer, and timestamps that start at 1700000000 and advance by 60 s per
//! commit. Commits are created with plumbing (`hash-object`, `update-index`
//! with a private `GIT_INDEX_FILE`, `write-tree`, `commit-tree`, `update-ref`)
//! so the same code path works for bare and normal repositories; normal
//! repositories additionally get their work tree synced so `git status` is
//! clean after every commit.
//!
//! Any git failure panics with the command line and git's stderr.

#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// First commit timestamp (unix seconds).
pub const EPOCH: u64 = 1_700_000_000;
/// Seconds between consecutive commits.
pub const TICK: u64 = 60;

pub const AUTHOR_NAME: &str = "Alice Example";
pub const AUTHOR_EMAIL: &str = "alice@example.com";

static INDEX_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A throw-away Git repository living in its own temporary directory.
///
/// `path` is the repository itself: the `.git`-less bare directory for bare
/// repositories, or the work tree root for normal ones.
pub struct TempRepo {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
    pub bare: bool,
    /// Timestamp used by the next commit / annotated tag.
    clock: u64,
    /// Set when the work tree can no longer be materialized (e.g. a file name
    /// the file system refuses); later work-tree syncs are then best-effort.
    lossy_worktree: bool,

    // Notable commits recorded by `rich()` (empty strings otherwise).
    /// The very first commit (README, sources, docs).
    pub root_oid: String,
    /// Merge of `feature/with-slash` into `main` (two parents).
    pub merge_oid: String,
    /// `docs/guide.md` renamed to `docs/GUIDE.md`.
    pub rename_oid: String,
    /// `notes/todo.txt` deleted.
    pub delete_oid: String,
    /// `scripts/run.sh` made executable (100644 -> 100755).
    pub mode_change_oid: String,
    /// Adds `assets/logo.png` and `bin/blob.bin`.
    pub binary_oid: String,
    /// Commit the annotated tag `v1.0.0` points at (the tag object itself is `v1_tag_oid`).
    pub v1_oid: String,
    /// Object id of the annotated tag `v1.0.0` itself.
    pub v1_tag_oid: String,
    /// Commit the `dup` branch points at.
    pub dup_branch_oid: String,
    /// Commit the `dup` tag points at (different from `dup_branch_oid`).
    pub dup_tag_oid: String,
    /// Tip of `feature/with-slash`.
    pub feature_oid: String,
}

/// Outcome of a git invocation.
pub struct GitOutput {
    pub status: std::process::ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl GitOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim_end().to_string()
    }
}

impl TempRepo {
    // ----------------------------------------------------------------------
    // Construction
    // ----------------------------------------------------------------------

    /// `git init` — a normal repository with a work tree.
    pub fn new_normal() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("gitcoat-fixture-")
            .tempdir()
            .expect("create temp dir");
        let path = dir.path().join("repo");
        fs::create_dir(&path).expect("create repo dir");
        let repo = Self::blank(dir, path, false);
        repo.git(&["init", "-q"]);
        repo
    }

    /// `git init --bare` (no commits).
    pub fn new_bare() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("gitcoat-fixture-")
            .tempdir()
            .expect("create temp dir");
        let path = dir.path().join("repo.git");
        fs::create_dir(&path).expect("create repo dir");
        let repo = Self::blank(dir, path, true);
        repo.git(&["init", "-q", "--bare"]);
        repo
    }

    /// A bare repository with no commits at all (alias for `new_bare`).
    pub fn empty_bare() -> Self {
        Self::new_bare()
    }

    fn blank(dir: tempfile::TempDir, path: PathBuf, bare: bool) -> Self {
        TempRepo {
            dir,
            path,
            bare,
            clock: EPOCH,
            lossy_worktree: false,
            root_oid: String::new(),
            merge_oid: String::new(),
            rename_oid: String::new(),
            delete_oid: String::new(),
            mode_change_oid: String::new(),
            binary_oid: String::new(),
            v1_oid: String::new(),
            v1_tag_oid: String::new(),
            dup_branch_oid: String::new(),
            dup_tag_oid: String::new(),
            feature_oid: String::new(),
        }
    }

    /// The `.git` directory of a normal repository, or the repository itself
    /// when bare.
    pub fn git_dir(&self) -> PathBuf {
        if self.bare {
            self.path.clone()
        } else {
            self.path.join(".git")
        }
    }

    // ----------------------------------------------------------------------
    // Running git
    // ----------------------------------------------------------------------

    fn base_command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd);
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_CEILING_DIRECTORIES",
            "GIT_COMMON_DIR",
            "GIT_EXTERNAL_DIFF",
            "GIT_PAGER",
            "GIT_EDITOR",
            "XDG_CONFIG_HOME",
        ] {
            cmd.env_remove(var);
        }
        let date = format!("{} +0000", self.clock);
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C.UTF-8")
            .env("GIT_AUTHOR_NAME", AUTHOR_NAME)
            .env("GIT_AUTHOR_EMAIL", AUTHOR_EMAIL)
            .env("GIT_COMMITTER_NAME", AUTHOR_NAME)
            .env("GIT_COMMITTER_EMAIL", AUTHOR_EMAIL)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .args([
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
                "-c",
                "core.autocrlf=false",
                "-c",
                "gc.auto=0",
                "-c",
                "protocol.file.allow=always",
            ]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// Run git in the repository; panic on failure.
    pub fn git<S: AsRef<OsStr>>(&self, args: &[S]) -> String {
        self.git_in(&self.path, args, None).stdout_str()
    }

    /// Run git in the repository with the given stdin; panic on failure.
    pub fn git_stdin<S: AsRef<OsStr>>(&self, args: &[S], stdin: &[u8]) -> String {
        self.git_in(&self.path, args, Some(stdin)).stdout_str()
    }

    /// Run git in the repository and return the outcome without panicking.
    pub fn try_git<S: AsRef<OsStr>>(&self, args: &[S]) -> GitOutput {
        self.run(&self.path, args, None, &[])
    }

    fn git_in<S: AsRef<OsStr>>(&self, cwd: &Path, args: &[S], stdin: Option<&[u8]>) -> GitOutput {
        let out = self.run(cwd, args, stdin, &[]);
        if !out.status.success() {
            panic_git(cwd, args, &out);
        }
        out
    }

    /// Run git with a private index file (for plumbing-based commits).
    fn git_index<S: AsRef<OsStr>>(
        &self,
        index: &Path,
        args: &[S],
        stdin: Option<&[u8]>,
    ) -> GitOutput {
        let extra = [("GIT_INDEX_FILE", index.as_os_str().to_os_string())];
        let out = self.run(&self.path, args, stdin, &extra);
        if !out.status.success() {
            panic_git(&self.path, args, &out);
        }
        out
    }

    fn run<S: AsRef<OsStr>>(
        &self,
        cwd: &Path,
        args: &[S],
        stdin: Option<&[u8]>,
        extra_env: &[(&str, OsString)],
    ) -> GitOutput {
        let mut cmd = self.base_command(cwd);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.args(args);
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd.spawn().expect("spawn git");
        if let Some(bytes) = stdin {
            let mut pipe = child.stdin.take().expect("stdin pipe");
            // Write on a thread so a full stdout pipe cannot deadlock us.
            let bytes = bytes.to_vec();
            let writer = std::thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            });
            let out = child.wait_with_output().expect("wait for git");
            let _ = writer.join();
            return GitOutput {
                status: out.status,
                stdout: out.stdout,
                stderr: out.stderr,
            };
        }
        let out = child.wait_with_output().expect("wait for git");
        GitOutput {
            status: out.status,
            stdout: out.stdout,
            stderr: out.stderr,
        }
    }

    fn tick(&mut self) {
        self.clock += TICK;
    }

    // ----------------------------------------------------------------------
    // Queries
    // ----------------------------------------------------------------------

    /// Resolve `spec` to a full object id; panics if it does not resolve.
    pub fn rev_parse(&self, spec: &str) -> String {
        self.try_rev_parse(spec)
            .unwrap_or_else(|| panic!("rev-parse {spec:?} did not resolve"))
    }

    /// Resolve `spec` to a full object id, or `None`.
    pub fn try_rev_parse(&self, spec: &str) -> Option<String> {
        let out = self.try_git(&["rev-parse", "--verify", "-q", "--end-of-options", spec]);
        if out.status.success() {
            Some(out.stdout_str())
        } else {
            None
        }
    }

    /// Commit id HEAD points at (panics for an empty repository).
    pub fn head_oid(&self) -> String {
        self.rev_parse("HEAD^{commit}")
    }

    /// Full name of the branch HEAD points at (e.g. `refs/heads/main`).
    pub fn head_ref(&self) -> String {
        self.git(&["symbolic-ref", "HEAD"])
    }

    fn zero_oid(&self) -> String {
        let len = match self.git(&["rev-parse", "--show-object-format"]).as_str() {
            "sha256" => 64,
            _ => 40,
        };
        "0".repeat(len)
    }

    // ----------------------------------------------------------------------
    // Commits (plumbing; works for bare and normal repositories)
    // ----------------------------------------------------------------------

    /// Write files (created or overwritten) and commit on HEAD's branch.
    /// Returns the new commit id.
    pub fn commit_files(&mut self, files: &[(&str, &[u8])], message: &str) -> String {
        let entries: Vec<IndexEntry> = files
            .iter()
            .map(|(p, b)| IndexEntry::file(p.as_bytes(), b))
            .collect();
        self.commit_entries("HEAD", &entries, message)
    }

    /// Remove paths and commit on HEAD's branch. Returns the new commit id.
    pub fn remove_files(&mut self, paths: &[&str], message: &str) -> String {
        let entries: Vec<IndexEntry> = paths
            .iter()
            .map(|p| IndexEntry::remove(p.as_bytes()))
            .collect();
        self.commit_entries("HEAD", &entries, message)
    }

    /// Like `commit_files`, but on an arbitrary branch (`refs/heads/x`)
    /// without touching HEAD.
    pub fn commit_files_on(
        &mut self,
        refname: &str,
        files: &[(&str, &[u8])],
        message: &str,
    ) -> String {
        let entries: Vec<IndexEntry> = files
            .iter()
            .map(|(p, b)| IndexEntry::file(p.as_bytes(), b))
            .collect();
        self.commit_entries(refname, &entries, message)
    }

    /// Rename `from` to `to` (same blob, same mode) on HEAD's branch.
    pub fn rename_file(&mut self, from: &str, to: &str, message: &str) -> String {
        let (mode, oid) = self.entry_at("HEAD", from);
        let entries = [
            IndexEntry::remove(from.as_bytes()),
            IndexEntry {
                path: to.as_bytes().to_vec(),
                mode,
                blob: Blob::Existing(oid),
            },
        ];
        self.commit_entries("HEAD", &entries, message)
    }

    /// Change the file mode of an existing path (`"100755"` / `"100644"`).
    pub fn chmod_file(&mut self, path: &str, mode: &str, message: &str) -> String {
        let (_, oid) = self.entry_at("HEAD", path);
        let entries = [IndexEntry {
            path: path.as_bytes().to_vec(),
            mode: mode.into(),
            blob: Blob::Existing(oid),
        }];
        self.commit_entries("HEAD", &entries, message)
    }

    /// Add a symbolic link entry (mode 120000) pointing at `target`.
    pub fn commit_symlink(&mut self, path: &str, target: &str, message: &str) -> String {
        let entries = [IndexEntry {
            path: path.as_bytes().to_vec(),
            mode: "120000".into(),
            blob: Blob::Content(target.as_bytes().to_vec()),
        }];
        self.commit_entries("HEAD", &entries, message)
    }

    /// Commit a file whose name contains a newline. Git stores such names in
    /// trees; returns whether the commit was created.
    pub fn add_newline_name_file(&mut self) -> bool {
        self.add_raw_name_file(
            b"new\nline.txt",
            b"newline name\n",
            "Add file with newline in name",
        )
    }

    /// Commit a file whose name is not valid UTF-8 (`bad\xff.txt`).
    /// Returns whether the commit was created.
    pub fn add_invalid_utf8_name_file(&mut self) -> bool {
        self.add_raw_name_file(
            b"bad\xff.txt",
            b"invalid utf-8 name\n",
            "Add file with invalid UTF-8 name",
        )
    }

    fn add_raw_name_file(&mut self, name: &[u8], content: &[u8], message: &str) -> bool {
        let entries = [IndexEntry::file(name, content)];
        // Git itself may refuse the name; the work tree is best-effort here.
        self.try_commit_entries("HEAD", &entries, message, true)
            .is_ok()
    }

    /// `(mode, blob oid)` of `path` in `treeish`; panics if missing.
    pub fn entry_at(&self, treeish: &str, path: &str) -> (String, String) {
        let out = self.git(&["ls-tree", "--end-of-options", treeish, "--", path]);
        let line = out
            .lines()
            .next()
            .unwrap_or_else(|| panic!("{path:?} not found in {treeish}"));
        // "<mode> SP <type> SP <oid> TAB <path>"
        let mut parts = line.split_whitespace();
        let mode = parts.next().expect("mode").to_string();
        let _kind = parts.next();
        let oid = parts.next().expect("oid").to_string();
        (mode, oid)
    }

    fn commit_entries(&mut self, refname: &str, entries: &[IndexEntry], message: &str) -> String {
        match self.try_commit_entries(refname, entries, message, false) {
            Ok(oid) => oid,
            Err(e) => panic!("{e}"),
        }
    }

    fn try_commit_entries(
        &mut self,
        refname: &str,
        entries: &[IndexEntry],
        message: &str,
        lenient_worktree: bool,
    ) -> Result<String, String> {
        let parent = self.try_rev_parse(&format!("{refname}^{{commit}}"));
        let index = self.temp_index();
        if let Some(parent) = &parent {
            self.git_index(&index, &["read-tree", "--end-of-options", parent], None);
        }
        let zero = self.zero_oid();
        let mut info: Vec<u8> = Vec::new();
        for e in entries {
            let (mode, oid) = match &e.blob {
                Blob::Removed => ("0".to_string(), zero.clone()),
                Blob::Existing(oid) => (e.mode.clone(), oid.clone()),
                Blob::Content(bytes) => {
                    let oid = self.git_stdin(&["hash-object", "-w", "--stdin"], bytes);
                    (e.mode.clone(), oid)
                }
            };
            info.extend_from_slice(mode.as_bytes());
            info.push(b' ');
            info.extend_from_slice(oid.as_bytes());
            info.push(b'\t');
            info.extend_from_slice(&e.path);
            info.push(0);
        }
        self.git_index(&index, &["update-index", "-z", "--index-info"], Some(&info));
        let tree = self.git_index(&index, &["write-tree"], None).stdout_str();
        let _ = fs::remove_file(&index);

        let mut args: Vec<String> = vec!["commit-tree".into(), tree, "-m".into(), message.into()];
        if let Some(parent) = &parent {
            args.push("-p".into());
            args.push(parent.clone());
        }
        let oid = self.git(&args);
        self.git(&["update-ref", "--end-of-options", refname, &oid]);
        self.tick();

        self.sync_worktree_if_head(refname, lenient_worktree)?;
        Ok(oid)
    }

    fn temp_index(&self) -> PathBuf {
        let n = INDEX_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.dir
            .path()
            .join(format!("index-{}-{n}", std::process::id()))
    }

    /// After a ref update, make a normal repository's index + work tree match
    /// HEAD so `git status` is clean.
    fn sync_worktree_if_head(&mut self, refname: &str, lenient: bool) -> Result<(), String> {
        if self.bare {
            return Ok(());
        }
        let head = self.try_git(&["symbolic-ref", "-q", "HEAD"]).stdout_str();
        if refname != "HEAD" && refname != head {
            return Ok(());
        }
        self.sync_worktree(lenient)
    }

    fn sync_worktree(&mut self, lenient: bool) -> Result<(), String> {
        if self.bare {
            return Ok(());
        }
        let out = self.try_git(&["reset", "-q", "--hard", "HEAD"]);
        if out.status.success() {
            return Ok(());
        }
        let msg = format!(
            "git reset --hard failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if lenient {
            // The commit exists; only the work tree could not be materialized
            // (e.g. a name the file system rejects). Remember that so later
            // syncs of the same repository stay best-effort.
            self.lossy_worktree = true;
            return Ok(());
        }
        if self.lossy_worktree {
            return Ok(());
        }
        Err(msg)
    }

    // ----------------------------------------------------------------------
    // Refs
    // ----------------------------------------------------------------------

    /// Create (or move) branch `name` at `from_oid`.
    pub fn branch(&mut self, name: &str, from_oid: &str) {
        let refname = format!("refs/heads/{name}");
        self.git(&["update-ref", "--end-of-options", &refname, from_oid]);
    }

    /// Lightweight tag.
    pub fn tag_light(&mut self, name: &str, oid: &str) {
        let refname = format!("refs/tags/{name}");
        self.git(&["update-ref", "--end-of-options", &refname, oid]);
    }

    /// Annotated tag; returns the tag object id.
    pub fn tag_annotated(&mut self, name: &str, oid: &str, message: &str) -> String {
        self.git(&["tag", "-a", "-m", message, "--end-of-options", name, oid]);
        self.tick();
        self.rev_parse(&format!("refs/tags/{name}"))
    }

    /// Point HEAD at `refname` (symbolic). For normal repositories the work
    /// tree is checked out to match.
    pub fn set_head(&mut self, refname: &str) {
        self.git(&["symbolic-ref", "HEAD", refname]);
        if !self.bare && self.try_rev_parse("HEAD^{commit}").is_some() {
            self.sync_worktree(false).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    /// Delete a branch or tag by full refname.
    pub fn delete_ref(&mut self, refname: &str) {
        self.git(&["update-ref", "-d", "--end-of-options", refname]);
    }

    /// Create a real merge commit (two parents) of `other_branch` into
    /// `into_branch`. Branch names are short (`main`, `feature/x`). The trees
    /// must merge without conflicts. Returns the merge commit id.
    pub fn merge(&mut self, into_branch: &str, other_branch: &str, message: &str) -> String {
        let into_ref = format!("refs/heads/{into_branch}");
        let other_ref = format!("refs/heads/{other_branch}");
        let ours = self.rev_parse(&format!("{into_ref}^{{commit}}"));
        let theirs = self.rev_parse(&format!("{other_ref}^{{commit}}"));
        let base = self.git(&["merge-base", "--end-of-options", &ours, &theirs]);

        let index = self.temp_index();
        self.git_index(
            &index,
            &[
                "read-tree",
                "-i",
                "-m",
                "--aggressive",
                "--end-of-options",
                &base,
                &ours,
                &theirs,
            ],
            None,
        );
        let tree = self.git_index(&index, &["write-tree"], None).stdout_str();
        let _ = fs::remove_file(&index);

        let oid = self.git(&[
            "commit-tree",
            &tree,
            "-m",
            message,
            "-p",
            &ours,
            "-p",
            &theirs,
        ]);
        self.git(&["update-ref", "--end-of-options", &into_ref, &oid]);
        self.tick();
        self.sync_worktree_if_head(&into_ref, false)
            .unwrap_or_else(|e| panic!("{e}"));
        oid
    }

    /// Clone the repository into a scratch directory, commit a new file there
    /// and push it back to HEAD's branch — simulating an external push.
    /// Returns the pushed commit id.
    pub fn push_new_commit_from_clone(&mut self, message: &str) -> String {
        let branch = self.head_ref();
        let short = branch
            .strip_prefix("refs/heads/")
            .unwrap_or(&branch)
            .to_string();
        if !self.bare {
            // Allow pushing into the checked-out branch of a normal repository.
            self.git(&["config", "receive.denyCurrentBranch", "updateInstead"]);
        }
        let n = INDEX_COUNTER.fetch_add(1, Ordering::Relaxed);
        let clone_dir = self.dir.path().join(format!("clone-{n}"));
        let repo_path = self.path.to_string_lossy().into_owned();
        let clone_path = clone_dir.to_string_lossy().into_owned();
        self.git_in(
            self.dir.path(),
            &[
                "clone",
                "-q",
                "--no-hardlinks",
                "--branch",
                &short,
                "--",
                &repo_path,
                &clone_path,
            ],
            None,
        );
        let file = format!("pushed-{n}.txt");
        fs::write(
            clone_dir.join(&file),
            format!("pushed from a clone ({message})\n"),
        )
        .expect("write pushed file");
        self.git_in(&clone_dir, &["add", "-A"], None);
        self.git_in(&clone_dir, &["commit", "-q", "-m", message], None);
        self.tick();
        let oid = self
            .git_in(&clone_dir, &["rev-parse", "HEAD"], None)
            .stdout_str();
        let refspec = format!("HEAD:{branch}");
        self.git_in(&clone_dir, &["push", "-q", "origin", &refspec], None);
        let _ = fs::remove_dir_all(&clone_dir);
        if !self.bare {
            self.sync_worktree(false).unwrap_or_else(|e| panic!("{e}"));
        }
        oid
    }

    /// `git pack-refs --all` + `git gc -q` on this fixture only.
    pub fn pack_refs_and_gc(&mut self) {
        self.git(&["pack-refs", "--all"]);
        self.git(&["gc", "-q"]);
    }

    /// Write an uncommitted `UNCOMMITTED.txt` and modify `README.md` in the
    /// work tree (normal repositories only).
    pub fn with_uncommitted_changes(self) -> Self {
        assert!(!self.bare, "with_uncommitted_changes needs a work tree");
        fs::write(self.path.join("UNCOMMITTED.txt"), b"not committed\n")
            .expect("write UNCOMMITTED.txt");
        let readme = self.path.join("README.md");
        let mut content = fs::read(&readme).unwrap_or_default();
        content.extend_from_slice(b"\nUNCOMMITTED CHANGE\n");
        fs::write(&readme, content).expect("modify README.md");
        self
    }

    // ----------------------------------------------------------------------
    // The standard rich fixture
    // ----------------------------------------------------------------------

    /// The standard fixture: a normal repository whose `main` has >= 120
    /// commits, a merge, a rename, a delete, a mode change, binary/odd files,
    /// branches `feature/with-slash` and `dup`, tags `v1.0.0` (annotated),
    /// `v0.9` and `dup` (lightweight). See the `*_oid` fields.
    pub fn rich() -> Self {
        let mut r = Self::new_normal();

        r.root_oid = r.commit_files(
            &[
                ("README.md", README_MD),
                ("src/main.rs", MAIN_RS),
                ("docs/guide.md", GUIDE_MD),
                ("empty.txt", b""),
            ],
            "Initial commit",
        );
        r.binary_oid = r.commit_files(
            &[("assets/logo.png", PNG_1X1), ("bin/blob.bin", BLOB_BIN)],
            "Add logo and binary blob",
        );
        r.commit_files(
            &[
                ("page.html", PAGE_HTML),
                ("icon.svg", ICON_SVG),
                (
                    "dir with spaces/中文文件.txt",
                    "中文内容\nsecond line\n".as_bytes(),
                ),
                ("-leading-dash.txt", b"leading dash\n"),
                ("weird#name%.txt", b"weird name\n"),
                ("notes/todo.txt", b"- write tests\n"),
                ("scripts/run.sh", b"#!/bin/sh\necho run\n"),
            ],
            "Add odd file names, HTML and SVG",
        );
        r.commit_symlink("link", "src/main.rs", "Add symlink to src/main.rs");
        let big = big_text();
        r.commit_files(&[("big.txt", &big)], "Add big.txt (> 1 MiB)");

        // Tags on older commits.
        r.v1_oid = r.binary_oid.clone();
        r.v1_tag_oid = r.tag_annotated(
            "v1.0.0",
            &r.binary_oid.clone(),
            "Release v1.0.0\n\nFirst tagged release.",
        );
        r.tag_light("v0.9", &r.root_oid.clone());

        // Feature branch that differs from main.
        let base = r.head_oid();
        r.branch("feature/with-slash", &base);
        r.feature_oid = r.commit_files_on(
            "refs/heads/feature/with-slash",
            &[("feature.txt", b"feature branch content\n")],
            "Add feature.txt on feature/with-slash",
        );

        // Many commits for pagination.
        r.counter_commits("refs/heads/main", 120);

        r.merge_oid = r.merge(
            "main",
            "feature/with-slash",
            "Merge branch 'feature/with-slash' into main",
        );
        r.rename_oid = r.rename_file(
            "docs/guide.md",
            "docs/GUIDE.md",
            "Rename docs/guide.md to docs/GUIDE.md",
        );
        r.delete_oid = r.remove_files(&["notes/todo.txt"], "Delete notes/todo.txt");
        r.mode_change_oid =
            r.chmod_file("scripts/run.sh", "100755", "Make scripts/run.sh executable");

        // `dup` branch and `dup` tag at different commits.
        r.dup_branch_oid = r.root_oid.clone();
        r.branch("dup", &r.root_oid.clone());
        r.dup_tag_oid = r.binary_oid.clone();
        r.tag_light("dup", &r.binary_oid.clone());

        r
    }

    /// Same content as `rich()`, as a bare repository (`git clone --bare`).
    pub fn rich_bare() -> Self {
        let normal = Self::rich();
        let dir = tempfile::Builder::new()
            .prefix("gitcoat-fixture-bare-")
            .tempdir()
            .expect("create temp dir");
        let path = dir.path().join("repo.git");
        let mut bare = Self::blank(dir, path, true);
        bare.clock = normal.clock;
        let src = normal.path.to_string_lossy().into_owned();
        let dst = bare.path.to_string_lossy().into_owned();
        bare.git_in(
            bare.dir.path(),
            &["clone", "-q", "--bare", "--no-hardlinks", "--", &src, &dst],
            None,
        );
        // A bare clone has no remote-tracking refs to worry about, but drop
        // the origin remote so the fixture is self-contained.
        let _ = bare.try_git(&["remote", "remove", "origin"]);
        bare.root_oid = normal.root_oid.clone();
        bare.merge_oid = normal.merge_oid.clone();
        bare.rename_oid = normal.rename_oid.clone();
        bare.delete_oid = normal.delete_oid.clone();
        bare.mode_change_oid = normal.mode_change_oid.clone();
        bare.binary_oid = normal.binary_oid.clone();
        bare.v1_oid = normal.v1_oid.clone();
        bare.v1_tag_oid = normal.v1_tag_oid.clone();
        bare.dup_branch_oid = normal.dup_branch_oid.clone();
        bare.dup_tag_oid = normal.dup_tag_oid.clone();
        bare.feature_oid = normal.feature_oid.clone();
        bare
    }

    /// Append `n` commits to `refname`, each rewriting `counter.txt`
    /// ("Bump counter to N"), via a single `git fast-import` run.
    pub fn counter_commits(&mut self, refname: &str, n: u32) {
        let parent = self.try_rev_parse(&format!("{refname}^{{commit}}"));
        let existing = self.try_git(&["cat-file", "-p", &format!("{refname}:counter.txt")]);
        let start: u32 = if existing.status.success() {
            existing.stdout_str().trim().parse().unwrap_or(0)
        } else {
            0
        };
        let mut stream = String::new();
        for i in 1..=n {
            let value = start + i;
            let msg = format!("Bump counter to {value}");
            let content = format!("{value}\n");
            stream.push_str(&format!("commit {refname}\n"));
            stream.push_str(&format!(
                "author {AUTHOR_NAME} <{AUTHOR_EMAIL}> {} +0000\n",
                self.clock
            ));
            stream.push_str(&format!(
                "committer {AUTHOR_NAME} <{AUTHOR_EMAIL}> {} +0000\n",
                self.clock
            ));
            stream.push_str(&format!("data {}\n{msg}\n", msg.len()));
            if i == 1
                && let Some(p) = &parent
            {
                stream.push_str(&format!("from {p}\n"));
            }
            stream.push_str(&format!(
                "M 100644 inline counter.txt\ndata {}\n{content}\n",
                content.len()
            ));
            self.tick();
        }
        stream.push_str("done\n");
        self.git_stdin(&["fast-import", "--quiet", "--done"], stream.as_bytes());
        self.sync_worktree_if_head(refname, false)
            .unwrap_or_else(|e| panic!("{e}"));
    }
}

// --------------------------------------------------------------------------
// Index entries
// --------------------------------------------------------------------------

enum Blob {
    Content(Vec<u8>),
    Existing(String),
    Removed,
}

struct IndexEntry {
    path: Vec<u8>,
    mode: String,
    blob: Blob,
}

impl IndexEntry {
    fn file(path: &[u8], content: &[u8]) -> Self {
        IndexEntry {
            path: path.to_vec(),
            mode: "100644".into(),
            blob: Blob::Content(content.to_vec()),
        }
    }

    fn remove(path: &[u8]) -> Self {
        IndexEntry {
            path: path.to_vec(),
            mode: "0".into(),
            blob: Blob::Removed,
        }
    }
}

fn panic_git<S: AsRef<OsStr>>(cwd: &Path, args: &[S], out: &GitOutput) -> ! {
    let shown: Vec<String> = args
        .iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    panic!(
        "git {} (in {}) failed with {}\nstdout: {}\nstderr: {}",
        shown.join(" "),
        cwd.display(),
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

// --------------------------------------------------------------------------
// Fixture content
// --------------------------------------------------------------------------

pub const README_MD: &[u8] = b"# GitCoat fixture

A **read-only** Git web UI test repository.

## Features

- Tree, blob and commit views
- Markdown rendering
  - nested item
- Syntax highlighting

## Tasks

- [x] done task
- [ ] pending task

| Name | Type | Notes |
| ---- | ---- | ----- |
| ref  | text | full refname |
| path | text | normalized |

Read the [docs](docs/guide.md), see the [source](src/main.rs) or go [up](../outside.md).
Jump to [features](#features). Logo: ![logo](assets/logo.png)

<script>alert(1)</script>

[x](javascript:alert(1))

```rust
fn main() { println!(\"hi\"); }
```
";

pub const GUIDE_MD: &[u8] = b"# Guide

See the [README](../README.md) and the [logo](../assets/logo.png).
";

pub const MAIN_RS: &[u8] = b"//! GitCoat fixture program.

use std::collections::HashMap;
use std::fmt;

/// A tiny key/value store used only for highlighting tests.
#[derive(Debug, Default, Clone)]
pub struct Store {
    items: HashMap<String, u64>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: &str, value: u64) -> Option<u64> {
        self.items.insert(key.to_string(), value)
    }

    pub fn get(&self, key: &str) -> Option<u64> {
        self.items.get(key).copied()
    }

    pub fn total(&self) -> u64 {
        self.items.values().sum()
    }
}

impl fmt::Display for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, \"Store({} items, total {})\", self.items.len(), self.total())
    }
}

fn main() {
    let mut store = Store::new();
    for (i, name) in [\"alpha\", \"beta\", \"gamma\"].iter().enumerate() {
        store.insert(name, (i as u64 + 1) * 10);
    }
    if let Some(v) = store.get(\"beta\") {
        println!(\"beta = {v}\"); // 20
    }
    println!(\"{store}\");
}
";

pub const PAGE_HTML: &[u8] = b"<!doctype html>
<html><head><title>fixture</title></head>
<body><script>alert('xss')</script><p>hello</p></body></html>
";

pub const ICON_SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" onload=\"alert(1)\"><circle cx=\"8\" cy=\"8\" r=\"6\"/></svg>
";

/// A valid 1x1 transparent PNG.
pub const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0x00, 0x02, 0x00,
    0x00, 0x05, 0x00, 0x01, 0xE2, 0x26, 0x05, 0x9B, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

/// Binary content with NUL bytes.
pub const BLOB_BIN: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE, 0x00, 0x00, 0x7F, 0x45, 0x4C, 0x46, 0x00, 0x10, 0x20, 0x30,
    0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x41, 0x42, 0x43, 0x00, 0x0A, 0x0D, 0x00, 0x00,
];

/// ~1.5 MiB of numbered lines.
pub fn big_text() -> Vec<u8> {
    let mut out = Vec::with_capacity(1_600_000);
    let mut i = 0u64;
    while out.len() < 1_572_864 {
        i += 1;
        out.extend_from_slice(
            format!("line {i:07}: the quick brown fox jumps over the lazy dog\n").as_bytes(),
        );
    }
    out
}
