# GitCoat

## Overview

GitCoat is a **single-repository, read-only** Git web browser: it presents one local Git repository (bare or a working checkout) the way a GitHub repository page does — directory tree, file preview (Markdown rendering, syntax highlighting), README, commit history and commit diffs. The look and feel is close to lightweight Git front-ends such as gong / Givy: light/dark theme, system fonts, thin borders, nothing fancy.

It is a **single self-contained binary**: the stylesheet and script are compiled in. It needs no database, no Node toolchain, no asset directory and no `git` executable at runtime.

### Explicit non-goals

GitCoat's scope is deliberately narrow. The following features are **not** provided:

- Accounts / login / permissions (put it behind a reverse proxy if you need access control)
- Multiple repositories, repository lists, organisations
- Issues, pull requests, CI, wiki
- A database or any persistent state
- Any Git write operation (clone, fetch, push, gc, modifying refs / config / hooks)
- HTTP Git transport (dumb / smart HTTP), SSH service
- Search, blame, branch comparison (compare), commit graph
- Git LFS
- A plugin system

## Tech stack and verified versions

| Component | Version | Notes |
|---|---|---|
| Rust | 1.98.1 (pinned in `rust-toolchain.toml`), edition 2024 | |
| [topcoat](https://crates.io/crates/topcoat) | `=0.8.1`, features limited to `router` / `serve` / `view` | Web framework; `view!` templates, routing |
| topcoat-cli | 0.8.1 | Development only, for `topcoat fmt` (formats `view!` bodies); not needed to build |
| [gix](https://crates.io/crates/gix) (gitoxide) | `=0.87.1`, pure Rust, no network features enabled | Reads refs, objects, trees, commits, diffs; **no git binary needed at runtime** |
| rushdown + ammonia | — | Markdown (GFM) rendering + HTML sanitisation |
| syntect | `default-fancy` (pure-Rust regex, no onig) | Highlighting for code and Markdown code blocks |
| clap / tracing / jiff / tokio / http | — | CLI and environment variables, logging, time formatting, async runtime |
| rust-i18n | 4 | UI translations compiled in from `locales/*.yml` |

## Building

```sh
scripts/build.sh
```

`scripts/build.sh` runs `cargo build --release --locked --bin gitcoat` and copies the result to:

```
dist/
└── gitcoat            # the whole deployment
```

Plain `cargo build --release` works just as well; the script only exists so the smoke test and the release workflow agree on where the binary lands.

### Single binary

`static/app.css` and `static/app.js` are embedded at compile time with `include_bytes!` (`src/app/static_files.rs`) and served from `/_static/app-<hash>.css` and `/_static/app-<hash>.js`. The `<hash>` is a 64-bit FNV-1a digest of the file's bytes, computed once at startup, so the URL changes whenever the file changes and the responses can be cached forever (`Cache-Control: public, max-age=31536000, immutable`). Only those two exact names are served; any other `/_static/…` path is a 404, and nothing is read from disk. To deploy, copy the one file `gitcoat` anywhere and run it — no asset directory, no working-directory requirement.

The `topcoat` CLI (`cargo install topcoat-cli --version 0.8.1 --locked`) is only needed by contributors, for `topcoat fmt`.

### Releases

Every `v*` tag is built by `.github/workflows/release.yml` and published on the [GitHub Releases](https://github.com/Tryanks/GitCoat/releases) page as one archive per target:

```
gitcoat-<version>-<target>.tar.gz     # Linux, macOS, FreeBSD
gitcoat-<version>-<target>.zip        # Windows
SHA256SUMS                            # checksums of every archive
```

Each archive contains just `gitcoat` (or `gitcoat.exe`), `LICENSE` and `README.md`. Targets: `x86_64` and `aarch64` for Linux (glibc and static musl), macOS and Windows, plus `armv7` (musl), `riscv64` and `loongarch64` Linux and `x86_64` FreeBSD. The musl archives are fully static and run on any Linux distribution.

To verify a download:

```sh
sha256sum -c --ignore-missing SHA256SUMS
```

## Running

```sh
dist/gitcoat --repo /srv/git/project.git --bind 127.0.0.1:3000 \
  --name project --description "Example repository" --clone-url git@git.example.com:project.git
```

| Flag | Environment variable | Default | Description |
|---|---|---|---|
| `--repo <PATH>` (required) | `GITCOAT_REPO` | — | Repository path: a bare directory, a checkout root, or its `.git` |
| `--bind <ADDR>` | `GITCOAT_BIND` | `127.0.0.1:3000` | Listen address (`ip:port`) |
| `--name <NAME>` | `GITCOAT_NAME` | Directory name with the `.git` suffix stripped | Repository name shown in the page title and header |
| `--description <TEXT>` | `GITCOAT_DESCRIPTION` | none | One-line description shown in the header |
| `--clone-url <URL>` | `GITCOAT_CLONE_URL` | none | Clone URL shown to visitors (with a copy button); display only |
| `--help` / `--version` | — | — | Help / version |

Precedence: **command-line flags > environment variables > defaults**.

Logs go to stderr via `tracing`; control the level with `RUST_LOG` (default `info`, e.g. `RUST_LOG=debug`). The process shuts down gracefully on Ctrl+C / SIGTERM. `/healthz` returns `200 ok` and can be used as a liveness probe.

## Bare and working repositories

- Both kinds are supported; GitCoat **only reads committed objects**.
- In a working repository (one with a working tree), uncommitted changes, untracked files and the staging area are never visible; pages always show a snapshot of some commit.
- A bare repository does not need to be, and never will be, checked out.
- `--repo` accepts: a bare repository directory, the checkout root of a working repository, or that checkout's `.git` directory. Subdirectories of a repository and non-repository directories are rejected with a clear error (startup fails).
- A repository with no commits starts fine; pages show a "No commits yet" empty state (plus the clone URL, if configured).

## Separation of duties between SSH and GitCoat

GitCoat only handles "viewing". Read/write transport for the repository is still done by the SSH setup you already have (`git@host:repo.git`):

- clone / fetch / push go over SSH and have nothing to do with GitCoat; GitCoat implements no Git protocol.
- GitCoat itself **never** clones, fetches or writes to the repository.
- `--clone-url` merely displays the address on the page with a copy button; GitCoat neither validates nor uses it.
- New commits, branches and tags pushed over SSH are visible on the next page refresh — refs are re-read on every request, with no caching; no restart is needed after an external `git gc` / `pack-refs` either.

## Read-only operation and permissions

- The repository directory can be mounted as a read-only filesystem (or `chmod -R a-w` as a whole) and GitCoat still works; the integration tests cover this.
- The repository is opened with gix's `isolated()` option: environment variables such as `GIT_DIR` / `GIT_WORK_TREE` are ignored and only the repository's own configuration is read.
- The repository is opened with **`Trust::Full`**: the path is explicitly given by the operator and is therefore treated as trusted, so the `git config --global --add safe.directory ...` step is unnecessary, and it does not matter if the files are owned by a different user than the one running GitCoat.
- GitCoat does not modify hooks, config or refs, and creates no temporary files, lock files or index inside the repository. The user running it only needs read access to the repository.
- Running it as a dedicated low-privilege user with read-only access to the repository directory is recommended.

## Caddy reverse proxy

GitCoat itself speaks plain HTTP only; TLS is left to the reverse proxy. Minimal Caddyfile (see `deploy/Caddyfile.example`):

```caddyfile
git.example.com {
	reverse_proxy 127.0.0.1:3000
}
```

- Caddy obtains and renews certificates automatically.
- **Only mounting at the root path `/` is supported**; sub-path deployments such as `https://example.com/git/` are not: all in-page links are root-relative (`/tree?...`) and the static files are served from `/_static/...`.
- When running GitCoat on the host, bind to `127.0.0.1` (the default) so only Caddy can reach it; in a container, use `--bind 0.0.0.0:3000` and point `reverse_proxy` at the container address.
- Pages do not depend on the `Host` header or proxy headers to build links, and use no absolute URLs, so changing the domain requires no configuration change.

## Pages and routes

| Route | Description |
|---|---|
| `GET /` | Repository home: root tree of the default ref + README |
| `GET /tree?ref=<ref>&path=<dir>` | Directory listing (ref selector, breadcrumbs, latest commit, README) |
| `GET /blob?ref=<ref>&path=<file>[&view=source]` | File preview; Markdown is rendered by default, `view=source` shows the source; text is highlighted, images are inlined, binaries get a notice |
| `GET /raw?ref=<ref>&path=<file>` | Raw file download (`text/plain` or `application/octet-stream` + `nosniff`; images are served inline with their real type) |
| `GET /commits?ref=<ref>&at=<oid>&page=N` | Commit history, 50 per page; `at` pins the starting point so pagination is stable |
| `GET /commit/<oid>` | Single commit: metadata, parents, file list and diff (merge commits are compared against the first parent) |
| `GET /lang?set=<tag>&back=<path>` | Pin the UI language in a cookie and redirect (303) back to `back`; see "Localization" |
| `GET /healthz` | `200 ok\n`, `text/plain` |
| `GET /_static/<name>` | The embedded stylesheet and script (content-hashed names, immutable caching); everything else is 404 |

The `ref` parameter accepts only a **full refname** (`refs/heads/x`, `refs/tags/x`) or a **full hexadecimal commit OID** (40 characters for SHA-1 / 64 for SHA-256); annotated tags are peeled to the commit they point to. Forms such as `main`, `HEAD`, abbreviated OIDs or `x^{tree}` always return 404 and are never parsed as revspecs. `path` must be a normalised relative path: no leading `/`, empty segments, `.` or `..`, otherwise 400.

When `ref` is omitted, the **default ref** is used, resolved in this order: `HEAD` (when it points at a valid commit) → `refs/heads/main` → `refs/heads/master` → the first other branch in name order → the first tag pointing at a commit. If none exist, the empty state is shown.

Each page resolves `ref` exactly once into a commit OID, and everything on the page (listing, README, latest commit) is based on that snapshot, so a concurrent push cannot make a page contradict itself.

## Localization

The interface is available in **English** (`en`, the default and the fallback) and **Simplified Chinese** (`zh-CN`). Only the UI copy is translated; commit messages, author names, ref names, file names, object ids and the `/healthz` body are always shown verbatim.

The language of a response is chosen per request, in this order:

1. the `gitcoat_lang` cookie, when it holds a supported tag (`en` or `zh-CN`; anything else is ignored);
2. the `Accept-Language` header, negotiated by q-value: `en` / `en-*` → English; `zh`, `zh-CN`, `zh-SG` and `zh-Hans*` → Simplified Chinese. Traditional Chinese ranges (`zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant*`) are **not** mapped to `zh-CN` — no Traditional Chinese translation exists yet, so a browser asking only for them gets English;
3. English.

The language switcher in the top bar links to `GET /lang?set=<tag>&back=<path>`, which validates `set` (400 for unknown tags), sets the cookie and answers `303 See Other` to `back`. `back` must be a root-relative path (`/…`, but not `//…` and no scheme); anything else redirects to `/`, so the endpoint cannot be used as an open redirect. The cookie is

```
Set-Cookie: gitcoat_lang=<tag>; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly
```

It is deliberately **not** marked `Secure`: GitCoat speaks plain HTTP behind a TLS-terminating proxy such as Caddy, and a `Secure` cookie could not be set through that hop. Every HTML page carries `Vary: Cookie, Accept-Language` and an `<html lang="…">` attribute matching the chosen language. `<time datetime>` values stay ISO 8601; only the human-readable relative text ("3 days ago" / "3 天前") is localised.

To add a language:

1. copy `locales/en.yml` to `locales/<tag>.yml` and translate every value (keep the keys and the `%{placeholders}`);
2. add a variant to `Lang` in `src/l10n.rs` with its `tag()` and `native_name()`, list it in `Lang::all()` and map the relevant `Accept-Language` ranges in `Lang::match_range`;
3. run `cargo test --locked`: `tests/l10n.rs` checks that every locale file defines exactly the same keys (with the same placeholders) and that every key used in the source exists.

## Preview and protection limits

All limits live in `src/limits.rs`:

| Constant | Value | Meaning |
|---|---|---|
| `GIT_CONCURRENCY` | 4 | Number of repository operations running at once (semaphore) |
| `GIT_TIMEOUT` | 10 s | Time limit for a single repository operation; 500 on timeout |
| `RAW_MAX_BYTES` | 64 MiB | Largest object any operation reads into memory; `/raw` returns 413 above this |
| `TEXT_PREVIEW_MAX_BYTES` | 1 MiB | Above this, files are no longer rendered as text, only offered for download |
| `TEXT_PREVIEW_MAX_LINES` | 10 000 | Maximum number of lines rendered in a text preview; the rest is truncated with a notice |
| `IMAGE_PREVIEW_MAX_BYTES` | 5 MiB | Above this, images are not inlined in the preview |
| `DIFF_MAX_BYTES` | 2 MiB | Total blob bytes read for a single commit diff; truncated beyond this |
| `DIFF_MAX_FILES` | 300 | Maximum number of files shown in a single commit diff |
| `TREE_MAX_ENTRIES` | 2000 | Maximum entries listed for a single directory; a truncation notice is shown beyond this |
| `COMMITS_PER_PAGE` | 50 | Commits per page in the history |
| `COMMITS_MAX_SKIP` | 50 000 | Maximum number of commits history pagination may skip; 404 beyond this |
| `README_MAX_BYTES` | 1 MiB | Size limit for READMEs rendered on directory pages |
| `REFS_MAX_ENTRIES` | 10 000 | Maximum number of branches / tags listed in the ref selector |

**About `RAW_MAX_BYTES`**: gix currently has no unified streaming read interface for loose + packed objects, so `/raw` first reads the object header to get the size, then reads the whole blob into memory before sending it. To protect server memory, files over 64 MiB are not served via `/raw`; a 413 with an explanation is returned instead, and such files should be obtained via `git clone`.

## Tests and checks

`.github/workflows/ci.yml` runs the formatting check, clippy and the tests on every push and pull request to `main`. Gates that must pass before committing:

```sh
cargo fmt --check
topcoat fmt && git diff --exit-code       # formats view! bodies; there should be no changes afterwards
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

End-to-end smoke test (runs `scripts/build.sh`; set `SKIP_BUILD=1` to reuse an existing `dist/`):

```sh
scripts/smoke.sh
```

It uses `git` to create a temporary repository (a few files, a Markdown README, a subdirectory, 3 commits, 1 tag), copies **only** `dist/gitcoat` into an empty temporary directory, starts it from **a different working directory**, then uses curl to check the status codes and key content of `/`, `/tree`, `/blob`, `/raw`, `/commits`, `/commit/<oid>`, `/healthz`, the `/_static/…` URLs found in the HTML (including their cache headers), and 400/404 responses one by one. If `caddy` is on the PATH, it also starts a temporary reverse proxy to verify that access through the proxy works and page links stay relative; otherwise it prints `Caddy not found: proxy check skipped`.

The integration tests (`tests/`) build fixture repositories in temporary directories via the `git` command line (fixed author and timestamps, `GIT_CONFIG_GLOBAL=/dev/null`, never touching global config or the network), then call the router in-process without listening on a port. **The tests need `git`; the application itself does not.**

## Known limitations

- Directory listings do not show the latest commit per file (GitHub-style per-row commit info), only the latest commit of the current ref.
- Tag version sorting is a simplified implementation (numeric segments compared numerically, everything else bytewise) and may differ from `git --sort=-version:refname` in edge cases.
- Directories with more than 2000 entries, diffs with more than 300 files / 2 MiB, and text files or READMEs over 1 MiB are truncated with a notice rather than shown in full.
- Deployment is only possible at the root path; sub-path reverse proxying is not supported.
- `/raw` is not streamed; large files (> 64 MiB) are refused for download.
- Commit history is in reverse commit-time order (similar to `git log --date-order`); no topological graph.
- Symbolic links only show their target path and are not followed; submodules only show their commit id.
- Entries whose file names are not valid UTF-8 are displayed (marked as unsupported name) without a link.
- No search, blame, compare, graph or LFS.
- The Caddy proxy check in `scripts/smoke.sh` runs only when `caddy` is installed locally; it is skipped in CI environments without caddy.
- Only English and Simplified Chinese are available; Traditional Chinese visitors get English.

## License

MIT, see `LICENSE`.
