#!/usr/bin/env python3
"""Fail when any tracked file carries a code comment that cites a planning
document.

`AGENTS.md` bans code comments that name a milestone, slice, task, design
id, review finding, planning-doc section, or a numbered test / failure-matrix
row / exit criterion. Those documents get archived and renumbered, so the
comment then lies. ADR references are permanent and stay allowed.

A cleanup pass brought the repository-wide count of these citations to zero,
so this gate scans every tracked `.rs`/`.wit`/`.toml`/`.ts` file's comment
lines directly rather than only a pull request's diff -- a citation
reintroduced anywhere, not just in new lines, now fails the build.

Usage:
    check-planning-refs.py

Exit 0 when clean, 1 when a citation is found.
"""

from __future__ import annotations

import re
import subprocess
import sys

# The seven families from docs/planning/code-quality/README.md, section
# "Finding planning references". The bare `§` family has an ADR carve-out
# applied in `citations()`.
PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("milestone", re.compile(r"\bM0[0-9][A-Z]?\b")),
    ("design id", re.compile(r"\bD-[0-9A-Z]{1,4}-[0-9]+\b")),
    ("slice", re.compile(r"\bSlice [A-Z]?[0-9]")),
    ("task id", re.compile(r"\b[A-Z][0-9][a-z]?\b(?=[’']s\b| (?:review|slice)\b)")),
    ("bare section ref", re.compile(r"§\s?[0-9]")),
    ("review finding", re.compile(r"\breview finding\b|\breview round [0-9]")),
    ("planning doc", re.compile(r"\b(?:status|task)\.md\b|\bimplementation-plan\b")),
    # A numbered test, failure-matrix row, or exit criterion -- each points at
    # a numbered table in a milestone doc and rots the same way. `test` needs a
    # 1-3 digit number so ordinary prose ("test the sequence") does not match;
    # a bare "test" with no number is fine.
    (
        "test or matrix row",
        re.compile(r"\b[Tt]est [0-9]{1,3}\b|\bmatrix row [0-9]|\bexit criteri(?:on|a)\b"),
    ),
]

# A `§` within short reach of `ADR-1234` points at a permanent record and is
# allowed. Mirrors the exclusion in triage-comments.py.
ADR_SECTION = re.compile(r"ADR-[0-9]{4}[^.]{0,12}§")

# Comment-line syntax, per extension. `.rs`/`.wit`/`.ts` share C-style line
# and block comments; `.toml` only has `#`.
C_STYLE = re.compile(r"^\s*(?://|/\*|\*)")
C_STYLE_PREFIX = re.compile(r"^\s*(?:///?!?|//!?|\*/?|/\*+!?)\s?")
HASH_STYLE = re.compile(r"^\s*#")
HASH_STYLE_PREFIX = re.compile(r"^\s*#!?\s?")

EXTENSIONS: dict[str, tuple[re.Pattern[str], re.Pattern[str]]] = {
    ".rs": (C_STYLE, C_STYLE_PREFIX),
    ".wit": (C_STYLE, C_STYLE_PREFIX),
    ".ts": (C_STYLE, C_STYLE_PREFIX),
    ".toml": (HASH_STYLE, HASH_STYLE_PREFIX),
}


def citations(prose: str) -> list[str]:
    """Return the labels of every banned citation family found in `prose`."""
    found: list[str] = []
    for label, pattern in PATTERNS:
        match = pattern.search(prose)
        if not match:
            continue
        if label == "bare section ref":
            window = prose[max(0, match.start() - 24) : match.end()]
            if ADR_SECTION.search(window):
                continue
        found.append(label)
    return found


def tracked_files() -> list[str]:
    globs = [f"*{ext}" for ext in EXTENSIONS]
    out = subprocess.run(
        ["git", "ls-files", *globs],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return out.splitlines()


def comment_lines(path: str) -> list[tuple[int, str]]:
    """(line number, raw text) for every comment line in `path`."""
    for ext, (is_comment, _) in EXTENSIONS.items():
        if path.endswith(ext):
            comment_re = is_comment
            break
    else:
        return []

    try:
        with open(path, encoding="utf-8") as handle:
            lines = handle.readlines()
    except (OSError, UnicodeDecodeError):
        return []

    return [(i, line) for i, line in enumerate(lines, start=1) if comment_re.match(line)]


def strip_prefix(path: str, raw: str) -> str:
    for ext, (_, prefix) in EXTENSIONS.items():
        if path.endswith(ext):
            return prefix.sub("", raw.strip())
    return raw.strip()


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print(__doc__, file=sys.stderr)
        return 2

    violations: list[tuple[str, int, str, list[str]]] = []
    for path in tracked_files():
        for line_no, raw in comment_lines(path):
            prose = strip_prefix(path, raw)
            kinds = citations(prose)
            if kinds:
                violations.append((path, line_no, raw.strip(), kinds))

    if not violations:
        print("planning-ref gate: no planning-document citations in the tree.")
        return 0

    print("planning-ref gate: a tracked file carries a comment line that cites a")
    print("planning document. AGENTS.md bans these -- the docs get archived and")
    print("renumbered, so the comment starts to lie. Rewrite each line to state the")
    print("constraint itself (docs/planning/code-quality/comment-convention.md).")
    print("ADR references are allowed.\n")
    for path, line_no, text, kinds in violations:
        print(f"  {path}:{line_no}  [{', '.join(sorted(set(kinds)))}]")
        print(f"    {text}")
    print(f"\n{len(violations)} offending line(s).")
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
