#!/usr/bin/env bash
# Build the release binary, bundle its assets, and assemble a self-contained
# `dist/` directory (`dist/gitcoat` + `dist/assets/`).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

command -v topcoat >/dev/null || {
    echo "error: the topcoat CLI is not installed (cargo install topcoat-cli --version 0.8.1 --locked)" >&2
    exit 1
}

cargo build --release --locked --bin gitcoat
# Scans the release binary for asset!() declarations and writes the bundle to
# the default location next to the executable: target/release/assets/.
topcoat asset bundle --release --bin gitcoat

test -f target/release/assets/manifest.toml

rm -rf dist
mkdir -p dist
cp target/release/gitcoat dist/gitcoat
cp -R target/release/assets dist/assets

echo "dist/ contents:"
find dist -type f | sort | sed 's/^/  /'
