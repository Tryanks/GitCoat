# AGENTS.md — Notes for GitCoat contributors and agents

## Product boundaries (do not cross)

GitCoat is a **single-repository, read-only** Git web browser. Adding any of the following is explicitly forbidden:
accounts/login, multiple repositories, issues/PRs/CI, a database, a Node/front-end toolchain, any Git write operation
(clone/fetch/push/gc/update-ref/config changes), HTTP Git transport (dumb/smart),
SSH service, search/blame/compare/graph/LFS/plugins. `--clone-url` is for display only.

## Tech stack (versions are pinned)

- Rust 1.98.1 (`rust-toolchain.toml`), edition 2024.
- `topcoat = "=0.8.1"`, features limited to `router`/`serve`/`view`; CLI `topcoat-cli 0.8.1` (only for `topcoat fmt`).
- Git access: `gix = "=0.87.1"` (pure Rust, no network features). git2 is **forbidden**, as is invoking the git command line from the application.
- Markdown: rushdown + ammonia; highlighting: syntect `default-fancy` (no onig).
- No JS framework, no CDN, no external scripts; `static/app.js` only handles theme switching, copy and dropdowns.
- The release artefact is **one self-contained binary**: nothing may be read from a directory next to the
  executable at runtime.

## Code layout

```
src/config.rs     CLI/environment variables (clap; precedence CLI > env > default)
src/limits.rs     All resource limit constants (single source of truth)
src/git/*         Repo handle (repo.rs), refs, tree, commit, diff, types, error
src/app/*         router/AppState (mod.rs), layout, home, tree, blob, raw, commits, commit, lang, url,
                  static_files (embedded app.css/app.js served at /_static/<hashed name>)
src/render/*      Markdown rendering/sanitisation, syntect highlighting (the only source of trusted HTML)
src/l10n.rs       UI languages (`Lang`), per-request negotiation, the `tr!` macro
locales/*.yml     UI copy per language (rust-i18n, compiled in)
static/           app.css, app.js (embedded with include_bytes!), icons/*.svg (reference drawings; the
                  inline SVGs live in src/app/components.rs)
tests/common/fixtures.rs   Temporary repository fixtures built with the git CLI (TempRepo)
```

## Conventions

- All Git access must go through the `Repo` API (`spawn_blocking` + Semaphore + timeout + iteration caps);
  page code never touches `gix::Repository` directly.
- Each page resolves `ref` exactly once → one commit OID; everything on the page is based on that snapshot.
- User-supplied paths are always validated through `RepoPath` (rejects `.`, `..`, empty segments, leading `/`, NUL);
  non-UTF-8 file names are displayed only, without links.
- `Unescaped::new_unchecked` may only be used for ammonia-sanitised Markdown and syntect output.
- URLs are always built with the constructors in `src/app/url.rs`; never hand-concatenate unencoded values.
- Static files: add them to `FILES` in `src/app/static_files.rs` (`include_bytes!` + content type) and link them
  with `static_url("name")`; never use topcoat's `asset!` / `AssetBundle` or serve from the file system. The
  served names are content-hashed (FNV-1a 64) so responses are `immutable`.
- Any new limit must go into `src/limits.rs` and be recorded in the README's "Preview and protection limits" table.
- Formatting: run `cargo fmt`, then `topcoat fmt` (formats `view!` bodies); both must be clean.
- Tests use the temporary repositories from `tests/common`; neither tests nor the application may read or write global git config or access the network.
- Do not suppress whole lint classes with `#![allow]`; fix the code.
- All UI copy goes through `tr!(lang, "key", ...)` (`src/l10n.rs`); never hard-code visible strings in `view!`.
  `locales/en.yml` and `locales/zh-CN.yml` must define the same keys (`tests/l10n.rs` enforces it).
  Never localise Git data (commit messages, names, refs, paths, OIDs) or `/healthz`. Pass the request's `Lang`
  explicitly; never call `rust_i18n::set_locale` (requests are concurrent). Strings the script needs go out as
  `data-*` attributes rendered from Rust.

## Quality gates (all must pass before committing)

```sh
cargo fmt --check
topcoat fmt && git diff --exit-code
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
bash -n scripts/smoke.sh && scripts/smoke.sh
```

`scripts/build.sh` produces `dist/gitcoat`; the smoke test copies that one file alone into an empty directory
and runs it from another cwd, which is what proves the binary is self-contained. Releases are built by
`.github/workflows/release.yml` (tags `v*`), CI by `.github/workflows/ci.yml`.
