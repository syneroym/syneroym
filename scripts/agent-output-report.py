#!/usr/bin/env python3
"""Summarize how much Bash output recent Claude Code sessions produced.

Reads this project's session transcripts from
``~/.claude/projects/<sanitized-cwd>/**/*.jsonl`` (Claude Code's own log
format, including subagent transcripts under `<session-uuid>/subagents/`),
matches each Bash tool call to its result, groups by a command key (the
first few whitespace-separated tokens after stripping a leading `cd <dir>
;`/`&&` or `VAR=value`, e.g. ``cargo test -p``), and prints a table sorted
by total output size. Use it during a code-quality
round, or whenever a session feels noisier than it should, to see which
commands are worth quieting next (see AGENTS.md's "Context Budget" rules).

Read-only: it only reads local transcript files and never sends anything
anywhere.

Usage:
    agent-output-report.py [--days N] [--top N]
"""

from __future__ import annotations

import argparse
import json
import re
import time
from dataclasses import dataclass
from pathlib import Path

# A leading `cd <dir>` followed by `;`, `&&`, or a plain newline (Claude
# Code often writes multi-line commands as "cd <dir>\n<rest>"), and a
# leading `VAR=value` assignment, stripped (repeatedly) before grouping --
# otherwise `cd /repo && git status` and a bare `git status` land in
# different rows.
_LEADING_CD_RE = re.compile(r"^\s*cd\s+\S+\s*(?:;|&&|\n)\s*")
_LEADING_ENV_VAR_RE = re.compile(r"^\s*[A-Za-z_][A-Za-z0-9_]*=\S*\s+")


def strip_command_prefix(command: str) -> str:
    stripped = command
    while True:
        without_cd = _LEADING_CD_RE.sub("", stripped, count=1)
        without_env = _LEADING_ENV_VAR_RE.sub("", without_cd, count=1)
        if without_env == stripped:
            return stripped
        stripped = without_env

# Matches Claude Code's own transcript-directory naming: the project's
# absolute path with every "/" replaced by "-" (e.g. "/Users/a/b" ->
# "-Users-a-b").
def transcripts_dir(project_root: Path) -> Path:
    sanitized = str(project_root.resolve()).replace("/", "-")
    return Path.home() / ".claude" / "projects" / sanitized


@dataclass
class CommandStats:
    calls: int = 0
    total_bytes: int = 0
    max_bytes: int = 0
    example: str = ""


def command_key(command: str, key_tokens: int) -> str:
    tokens = strip_command_prefix(command).split()
    return " ".join(tokens[:key_tokens]) if tokens else "(empty)"


def result_size(tool_use_result: object, content: object) -> int:
    """Prefer `toolUseResult.stdout`/`stderr` (what the agent actually saw);
    fall back to the raw `tool_result` content block."""
    if isinstance(tool_use_result, dict) and "stdout" in tool_use_result:
        stdout = tool_use_result.get("stdout") or ""
        stderr = tool_use_result.get("stderr") or ""
        if isinstance(stdout, str) and isinstance(stderr, str):
            return len(stdout) + len(stderr)
    if isinstance(content, str):
        return len(content)
    return len(json.dumps(content))


def iter_transcript_lines(path: Path):
    with path.open(encoding="utf-8", errors="ignore") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                continue


def scan_transcript(path: Path, stats: dict[str, CommandStats], key_tokens: int) -> None:
    bash_tool_use_ids: dict[str, str] = {}

    for entry in iter_transcript_lines(path):
        entry_type = entry.get("type")

        if entry_type == "assistant":
            for block in entry.get("message", {}).get("content", []) or []:
                if isinstance(block, dict) and block.get("type") == "tool_use" and block.get("name") == "Bash":
                    command = block.get("input", {}).get("command", "")
                    if command:
                        bash_tool_use_ids[block["id"]] = command

        elif entry_type == "user":
            content = entry.get("message", {}).get("content")
            if not isinstance(content, list):
                continue
            for block in content:
                if not (isinstance(block, dict) and block.get("type") == "tool_result"):
                    continue
                command = bash_tool_use_ids.get(block.get("tool_use_id", ""))
                if command is None:
                    continue
                size = result_size(entry.get("toolUseResult"), block.get("content"))
                key = command_key(command, key_tokens)
                bucket = stats.setdefault(key, CommandStats())
                bucket.calls += 1
                bucket.total_bytes += size
                if size > bucket.max_bytes:
                    bucket.max_bytes = size
                    bucket.example = command


def collect_stats(root: Path, days: int, key_tokens: int) -> dict[str, CommandStats]:
    stats: dict[str, CommandStats] = {}
    cutoff = time.time() - days * 86400
    if not root.is_dir():
        return stats
    # rglob, not glob: subagent transcripts live under
    # <session-uuid>/subagents/*.jsonl, not directly in `root`, and they
    # carry their own Bash tool calls worth counting.
    for path in root.rglob("*.jsonl"):
        if path.stat().st_mtime < cutoff:
            continue
        scan_transcript(path, stats, key_tokens)
    return stats


def print_table(stats: dict[str, CommandStats], top: int) -> None:
    if not stats:
        print("No Bash tool calls found in the scanned transcripts.")
        return

    rows = sorted(stats.items(), key=lambda kv: kv[1].total_bytes, reverse=True)[:top]
    key_width = max(len("command"), *(len(key) for key, _ in rows))

    header = f"{'command':<{key_width}}  {'calls':>6}  {'total KB':>10}  {'largest KB':>11}"
    print(header)
    print("-" * len(header))
    for key, s in rows:
        print(
            f"{key:<{key_width}}  {s.calls:>6}  {s.total_bytes / 1024:>10.1f}  {s.max_bytes / 1024:>11.1f}"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--days", type=int, default=14, help="how many days of transcripts to scan (default: 14)")
    parser.add_argument("--top", type=int, default=25, help="how many command groups to print (default: 25)")
    parser.add_argument(
        "--key-tokens", type=int, default=3, help="whitespace tokens used to group commands (default: 3)"
    )
    args = parser.parse_args()

    project_root = Path.cwd()
    root = transcripts_dir(project_root)
    stats = collect_stats(root, args.days, args.key_tokens)

    print(f"Scanned {root} (last {args.days} day(s))")
    print_table(stats, args.top)


if __name__ == "__main__":
    main()
