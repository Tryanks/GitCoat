#!/usr/bin/env bash
# Build the release binary and place it at `dist/gitcoat`. The stylesheet and
# script are compiled into the executable, so that single file is the whole
# deployment.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

cargo build --release --locked --bin gitcoat

rm -rf dist
mkdir -p dist
cp target/release/gitcoat dist/gitcoat

echo "dist/ contents:"
find dist -type f | sort | sed 's/^/  /'
