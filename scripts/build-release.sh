#!/usr/bin/env bash
set -euo pipefail

# Default to host target or allow overriding
TARGET="${1:-}"

if [ -n "$TARGET" ]; then
  echo "Building for target: $TARGET"
  if [ "$TARGET" = "aarch64-unknown-linux-gnu" ]; then
    if command -v cross >/dev/null 2>&1; then
      cross build --release --target "$TARGET" -p syneroym-substrate -p roymctl
    else
      echo "cross is required for target $TARGET but not installed. Installing..."
      cargo install cross --git https://github.com/cross-rs/cross
      cross build --release --target "$TARGET" -p syneroym-substrate -p roymctl
    fi
  elif [ "$TARGET" = "macOS-universal" ]; then
    rustup target add x86_64-apple-darwin aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin -p syneroym-substrate -p roymctl
    cargo build --release --target aarch64-apple-darwin -p syneroym-substrate -p roymctl
    mkdir -p target/release-universal
    lipo -create target/x86_64-apple-darwin/release/syneroym-substrate target/aarch64-apple-darwin/release/syneroym-substrate -output target/release-universal/syneroym-substrate
    lipo -create target/x86_64-apple-darwin/release/roymctl target/aarch64-apple-darwin/release/roymctl -output target/release-universal/roymctl
  else
    rustup target add "$TARGET" || true
    cargo build --release --target "$TARGET" -p syneroym-substrate -p roymctl
  fi
else
  # If no target specified, do standard build for current target
  cargo build --release -p syneroym-substrate -p roymctl
fi
