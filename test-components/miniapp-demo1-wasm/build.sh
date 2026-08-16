#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

if [ -d "client" ]; then
    echo "Building client assets..."
    (cd client && npm install && npm run build)
fi

echo "Building WASM component..."
cargo component build --release --target wasm32-wasip2
