#!/bin/sh
# PreToolUse/Bash guard: `cargo test -p <crate> <filter>` builds and runs
# every test binary in the crate one at a time and prints an empty "running
# 0 tests" block for each one it isn't targeting (~13 KB for
# syneroym-substrate's ~40 binaries, on every invocation). `cargo nextest
# run` runs the same suite in one parallel pool and prints ~1 KB. Keep
# `cargo test` for doctests, which nextest does not run.

command=$(cat | jq -r '.tool_input.command // empty')

# Anchored to where a command actually starts (start of string, or after
# ;/&/|/(  possibly with a VAR=value prefix), not just "cargo test"
# appearing anywhere -- otherwise this fires on `git commit -m "use cargo
# test for doctests"` or `echo "run cargo test later"`. `t` is cargo's own
# built-in short name for `test`.
if printf '%s\n' "$command" \
    | grep -Eq '(^|[;&|(]\s*)([A-Za-z_][A-Za-z0-9_]*=\S*\s+)*cargo(\s+\+\S+)?\s+(test|t)(\s|$)' \
    && ! printf '%s\n' "$command" | grep -Eq -- '(^|\s)--doc(\s|$)'; then
  reason='Use `cargo nextest run -p <crate> [--test <file>] <filter>` instead of `cargo test` -- it runs the same tests in one parallel pool instead of one binary at a time, with far less output. `cargo test` is reserved for doctests: `cargo test --workspace --doc`.'
  jq -n --arg reason "$reason" \
    '{hookSpecificOutput: {hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: $reason}}'
  exit 0
fi

exit 0
