#!/usr/bin/env python3
"""Fail when a diff ADDS a code comment that cites a planning document.

`AGENTS.md` bans code comments that name a milestone, slice, task, design
id, review finding, or planning-doc section. Those documents get archived
and renumbered, so the comment then lies. ADR references are permanent and
stay allowed.

The repository still carries about 1,200 known violations that a separate
round is cleaning, so this gate looks ONLY at the lines a pull request
adds. A full-tree scan is a later step (progress.md row G6).

Usage:
    check-planning-refs.py <base-ref>

`<base-ref>` is the commit the pull request branched from. Only added (`+`)
lines in `git diff <base-ref>...HEAD -- '*.rs'` are inspected. Exit 0 when
clean, 1 when a new citation is found, 2 on a usage error.
"""

from __future__ import annotations

import re
import subprocess
import sys

# The seven patterns from docs/planning/code-quality/README.md, section
# "Finding planning references". Each entry is (label, compiled regex).
# The bare `§` family has an ADR carve-out applied in `citations()`.
PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("milestone", re.compile(r"\bM0[0-9][A-Z]?\b")),
    ("design id", re.compile(r"\bD-[0-9A-Z]{1,4}-[0-9]+\b")),
    ("slice", re.compile(r"\bSlice [A-Z]?[0-9]")),
    ("task id", re.compile(r"\b[A-Z][0-9][a-z]?\b(?=[’']s\b| (?:review|slice)\b)")),
    ("bare section ref", re.compile(r"§\s?[0-9]")),
    ("review finding", re.compile(r"\breview finding\b|\breview round [0-9]")),
    ("planning doc", re.compile(r"\b(?:status|task)\.md\b|\bimplementation-plan\b")),
]

# A `§` within short reach of `ADR-1234` points at a permanent record and is
# allowed. Mirrors the exclusion in triage-comments.py.
ADR_SECTION = re.compile(r"ADR-[0-9]{4}[^.]{0,12}§")

# A Rust source line that is a comment: line/doc comments and the opening,
# continuation, or closing line of a block comment.
COMMENT_LINE = re.compile(r"^\s*(?://|/\*|\*)")
COMMENT_PREFIX = re.compile(r"^\s*(?:///?!?|//!?|\*/?|/\*+!?)\s?")

HUNK = re.compile(r"^@@ -[0-9,]+ \+([0-9]+)(?:,[0-9]+)? @@")


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


def added_comment_lines(base_ref: str) -> list[tuple[str, int, str]]:
    """(file, new line number, raw text) for every added `.rs` comment line."""
    diff = subprocess.run(
        ["git", "diff", "--unified=0", "--no-color", f"{base_ref}...HEAD", "--", "*.rs"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout

    out: list[tuple[str, int, str]] = []
    path: str | None = None
    new_line = 0
    for line in diff.split("\n"):
        if line.startswith("+++ b/"):
            path = line[6:]
        elif line.startswith("+++ "):
            path = None
        elif line.startswith("@@"):
            m = HUNK.match(line)
            new_line = int(m.group(1)) if m else 0
        elif line.startswith("+") and not line.startswith("+++"):
            content = line[1:]
            if path and COMMENT_LINE.match(content):
                out.append((path, new_line, content))
            new_line += 1
    return out


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    base_ref = argv[1]

    violations: list[tuple[str, int, str, list[str]]] = []
    for path, line_no, raw in added_comment_lines(base_ref):
        prose = COMMENT_PREFIX.sub("", raw).strip()
        kinds = citations(prose)
        if kinds:
            violations.append((path, line_no, raw.strip(), kinds))

    if not violations:
        print("planning-ref gate: no planning-document citations added.")
        return 0

    print("planning-ref gate: this diff adds comment lines that cite a planning")
    print("document. AGENTS.md bans these -- the docs get archived and renumbered,")
    print("so the comment starts to lie. Rewrite each line to state the constraint")
    print("itself (docs/planning/code-quality/comment-convention.md). ADR")
    print("references are allowed.\n")
    for path, line_no, text, kinds in violations:
        print(f"  {path}:{line_no}  [{', '.join(sorted(set(kinds)))}]")
        print(f"    {text}")
    print(f"\n{len(violations)} offending line(s).")
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
