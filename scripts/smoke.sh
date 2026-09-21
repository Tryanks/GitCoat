#!/usr/bin/env bash
# End-to-end smoke test against the real binary: build (unless SKIP_BUILD=1),
# create a throw-away repository with the git CLI, start `dist/gitcoat` from a
# different working directory, and check every route over HTTP. When `caddy`
# is on PATH the same checks run through a reverse proxy as well.
#
#   scripts/smoke.sh              # build + test
#   SKIP_BUILD=1 scripts/smoke.sh # reuse an existing dist/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------
pass=0
fail=0
app_pid=""
caddy_pid=""
TMP="$(mktemp -d "${TMPDIR:-/tmp}/gitcoat-smoke.XXXXXX")"

cleanup() {
    local status=$?
    [ -n "$caddy_pid" ] && kill "$caddy_pid" 2>/dev/null || true
    [ -n "$app_pid" ] && kill "$app_pid" 2>/dev/null || true
    [ -n "$caddy_pid" ] && wait "$caddy_pid" 2>/dev/null || true
    [ -n "$app_pid" ] && wait "$app_pid" 2>/dev/null || true
    if [ "$status" -ne 0 ] && [ -f "$TMP/gitcoat.log" ]; then
        echo "--- gitcoat log ---"
        cat "$TMP/gitcoat.log"
    fi
    rm -rf "$TMP"
}
trap cleanup EXIT

ok() {
    pass=$((pass + 1))
    echo "PASS  $1"
}
ko() {
    fail=$((fail + 1))
    echo "FAIL  $1"
}

free_port() {
    if command -v python3 >/dev/null; then
        python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
        return
    fi
    local port
    while :; do
        port=$((20000 + RANDOM % 20000))
        if ! (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
            echo "$port"
            return
        fi
    done
}

# fetch URL -> sets $status, $body_file, $headers_file
fetch() {
    body_file="$TMP/body"
    headers_file="$TMP/headers"
    status="$(curl -sS -o "$body_file" -D "$headers_file" -w '%{http_code}' "$1" || echo 000)"
}

header_is() {
    # header_is NAME EXPECTED-PREFIX (case-insensitive)
    grep -i "^$1: *$2" "$headers_file" >/dev/null
}

# check NAME URL EXPECTED_STATUS [needle...]
# Passes when the status matches and every needle is found in the body.
check() {
    local name="$1" url="$2" want="$3"
    shift 3
    fetch "$url"
    if [ "$status" != "$want" ]; then
        ko "$name: expected HTTP $want, got $status ($url)"
        return
    fi
    local needle
    for needle in "$@"; do
        if ! grep -F -q -- "$needle" "$body_file"; then
            ko "$name: body lacks '$needle' ($url)"
            return
        fi
    done
    ok "$name ($want)"
}

# check_absent NAME URL needle  — the body must NOT contain the needle
check_absent() {
    local name="$1" url="$2" needle="$3"
    fetch "$url"
    if grep -F -q -- "$needle" "$body_file"; then
        ko "$name: body contains '$needle' ($url)"
    else
        ok "$name"
    fi
}

wait_for_healthz() {
    local base="$1" i
    for i in $(seq 1 100); do
        if curl -sf "$base/healthz" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    return 1
}

# ---------------------------------------------------------------------------
# 1. build
# ---------------------------------------------------------------------------
if [ "${SKIP_BUILD:-0}" != "1" ]; then
    echo "== building (scripts/build.sh)"
    "$ROOT/scripts/build.sh"
fi
test -x "$ROOT/dist/gitcoat" || { echo "error: dist/gitcoat missing (run without SKIP_BUILD)"; exit 1; }
test -f "$ROOT/dist/assets/manifest.toml" || { echo "error: dist/assets/manifest.toml missing"; exit 1; }

# ---------------------------------------------------------------------------
# 2. fixture repository (git CLI only, isolated from any user config)
# ---------------------------------------------------------------------------
echo "== creating fixture repository"
REPO="$TMP/smoke-repo"
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_DATE="1700000000 +0000" GIT_COMMITTER_DATE="1700000000 +0000"
g() {
    git -C "$REPO" -c user.name="Smoke Tester" -c user.email="smoke@example.com" \
        -c commit.gpgsign=false -c tag.gpgsign=false "$@"
}
mkdir -p "$REPO"
g init -q -b main
mkdir -p "$REPO/src" "$REPO/docs"
cat >"$REPO/README.md" <<'EOF'
# Smoke test repository

This README is **Markdown** with a [link](docs/notes.txt) and code:

```rust
fn main() { println!("smoke"); }
```
EOF
printf 'fn main() {\n    println!("hello from smoke");\n}\n' >"$REPO/src/main.rs"
g add -A
g commit -q -m "First commit"
printf 'notes line one\n' >"$REPO/docs/notes.txt"
g add -A
g commit -q -m "Second commit: add docs"
printf 'notes line one\nnotes line two\n' >"$REPO/docs/notes.txt"
g add -A
g commit -q -m "Third commit: extend notes"
g tag -a v1.0.0 -m "Release 1.0.0"
HEAD_OID="$(g rev-parse HEAD)"
echo "   repo: $REPO (HEAD $HEAD_OID)"

# ---------------------------------------------------------------------------
# 3. deploy copy + start from a different cwd
# ---------------------------------------------------------------------------
DEPLOY="$TMP/deploy"
mkdir -p "$DEPLOY"
cp -R "$ROOT/dist" "$DEPLOY/dist"

APP_PORT="$(free_port)"
BASE="http://127.0.0.1:$APP_PORT"
echo "== starting $DEPLOY/dist/gitcoat on $BASE (cwd /tmp)"
(cd /tmp && exec "$DEPLOY/dist/gitcoat" --repo "$REPO" --bind "127.0.0.1:$APP_PORT" \
    --description "smoke description" --clone-url "git@example.com:smoke/repo.git") \
    >"$TMP/gitcoat.log" 2>&1 &
app_pid=$!
if ! wait_for_healthz "$BASE"; then
    echo "error: gitcoat did not become healthy"
    exit 1
fi

# ---------------------------------------------------------------------------
# 4. checks against the app
# ---------------------------------------------------------------------------
echo "== checking routes"
check "GET /healthz" "$BASE/healthz" 200 "ok"
fetch "$BASE/healthz"
if header_is content-type 'text/plain'; then ok "/healthz is text/plain"; else ko "/healthz content-type"; fi

check "GET /" "$BASE/" 200 "smoke-repo" "README.md" "src" "Smoke test repository" "smoke description"
fetch "$BASE/"
if header_is content-type 'text/html'; then ok "/ is text/html"; else ko "/ content-type"; fi
HOME_HTML="$TMP/home.html"
cp "$body_file" "$HOME_HTML"

check "GET /tree (subdir)" "$BASE/tree?ref=refs%2Fheads%2Fmain&path=src" 200 "main.rs" "breadcrumb"
check "GET /tree (tag)" "$BASE/tree?ref=refs%2Ftags%2Fv1.0.0" 200 "README.md"
check "GET /tree (bad path)" "$BASE/tree?path=..%2Fx" 400 "Bad request"
check "GET /tree (unknown ref)" "$BASE/tree?ref=refs%2Fheads%2Fnope" 404 "Ref not found"
check "GET /nope" "$BASE/nope" 404 "Page not found"

# Assets: URLs come from the served HTML, so they match this exact build.
CSS_URL="$(grep -oE 'href="/_topcoat/assets/[^"]+\.css"' "$HOME_HTML" | head -1 | sed -E 's/^href="//; s/"$//')"
JS_URL="$(grep -oE 'src="/_topcoat/assets/[^"]+\.js"' "$HOME_HTML" | head -1 | sed -E 's/^src="//; s/"$//')"
if [ -n "$CSS_URL" ]; then
    check "GET css asset" "$BASE$CSS_URL" 200 ":root"
    if header_is content-type 'text/css'; then ok "css content-type"; else ko "css content-type"; fi
else
    ko "no stylesheet URL found in /"
fi
if [ -n "$JS_URL" ]; then
    check "GET js asset" "$BASE$JS_URL" 200 "gitcoat-theme"
    if header_is content-type 'text/javascript'; then ok "js content-type"; else ko "js content-type"; fi
else
    ko "no script URL found in /"
fi
check "asset traversal" "$BASE/_topcoat/assets/../../Cargo.toml" 404

# Routes provided by the blob/raw and commits/commit pages.
check "GET /blob (markdown)" "$BASE/blob?ref=refs%2Fheads%2Fmain&path=README.md" 200 "Smoke test repository" "markdown-body"
check "GET /blob (source)" "$BASE/blob?ref=refs%2Fheads%2Fmain&path=src%2Fmain.rs" 200 "hello from smoke" "L1"
check "GET /raw" "$BASE/raw?ref=refs%2Fheads%2Fmain&path=src%2Fmain.rs" 200 'println!("hello from smoke")'
if header_is content-type 'text/plain'; then ok "raw content-type text/plain"; else ko "raw content-type"; fi
if header_is x-content-type-options 'nosniff'; then ok "raw nosniff"; else ko "raw X-Content-Type-Options"; fi
check "GET /commits" "$BASE/commits?ref=refs%2Fheads%2Fmain" 200 "Third commit: extend notes" "First commit"
check "GET /commit/<oid>" "$BASE/commit/$HEAD_OID" 200 "Third commit: extend notes" "notes line two" "docs/notes.txt"

# ---------------------------------------------------------------------------
# 5. optional reverse proxy check through Caddy
# ---------------------------------------------------------------------------
if command -v caddy >/dev/null; then
    CADDY_PORT="$(free_port)"
    CADDY_BASE="http://127.0.0.1:$CADDY_PORT"
    cat >"$TMP/Caddyfile" <<EOF
{
	admin off
}
http://127.0.0.1:$CADDY_PORT {
	reverse_proxy 127.0.0.1:$APP_PORT
}
EOF
    echo "== starting caddy on $CADDY_BASE"
    caddy run --config "$TMP/Caddyfile" --adapter caddyfile >"$TMP/caddy.log" 2>&1 &
    caddy_pid=$!
    if wait_for_healthz "$CADDY_BASE"; then
        check "GET / via caddy" "$CADDY_BASE/" 200 "smoke-repo" "README.md"
        check_absent "links stay relative via caddy" "$CADDY_BASE/" "127.0.0.1:$APP_PORT"
        check "GET /tree via caddy" "$CADDY_BASE/tree?ref=refs%2Fheads%2Fmain&path=docs" 200 "notes.txt"
        if [ -n "$CSS_URL" ]; then
            check "GET css via caddy" "$CADDY_BASE$CSS_URL" 200 ":root"
        fi
    else
        ko "caddy did not become healthy"
        cat "$TMP/caddy.log"
    fi
else
    echo "Caddy not found: proxy check skipped"
fi

# ---------------------------------------------------------------------------
# 6. summary
# ---------------------------------------------------------------------------
echo
echo "== summary: $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
    echo "SMOKE FAIL"
    exit 1
fi
echo "SMOKE PASS"
