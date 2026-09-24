//! Quiet, single-command runner for the AGENTS.md completion checklist.
//!
//! Each gate's full stdout/stderr goes to `target/gates/<gate>.log`, and only
//! a one-line summary (or, on failure, a short tail of that log) is printed.
//! Adding a checklist command means adding one `Gate` entry to `GATES` below.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};

struct Gate {
    name: &'static str,
    /// Skipped, along with the other docs-only-skippable gates, when the
    /// change under verification touches only `*.md` files and `docs/`.
    skip_on_docs_only: bool,
    /// argv (program, then its args) run in sequence; the gate fails at the
    /// first non-zero exit, and later commands in the list are not run.
    commands: &'static [&'static [&'static str]],
}

const GATES: &[Gate] = &[
    Gate {
        name: "fmt",
        skip_on_docs_only: false,
        commands: &[&["cargo", "+nightly", "fmt", "--all", "--", "--check"]],
    },
    Gate {
        name: "clippy",
        skip_on_docs_only: false,
        commands: &[&[
            "cargo",
            "clippy",
            "-q",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ]],
    },
    Gate {
        name: "file-lengths",
        skip_on_docs_only: false,
        commands: &[&["cargo", "xtask", "check-file-lengths"]],
    },
    Gate {
        name: "lint-suppressions",
        skip_on_docs_only: false,
        commands: &[&["cargo", "xtask", "check-lint-suppressions"]],
    },
    Gate {
        name: "duplication",
        skip_on_docs_only: false,
        commands: &[&["cargo", "xtask", "check-duplication"]],
    },
    Gate {
        name: "roym-deps",
        skip_on_docs_only: false,
        commands: &[&["cargo", "xtask", "check-roym-deps"]],
    },
    Gate {
        name: "planning-refs",
        skip_on_docs_only: false,
        commands: &[&["python3", ".github/scripts/check-planning-refs.py"]],
    },
    Gate {
        name: "nextest",
        skip_on_docs_only: true,
        // Some suites (e.g. `dual_build_parity`) load pre-built
        // `wasm32-wasip2` artifacts straight from disk; nothing in `cargo
        // nextest run` builds them, so build them first, same as CI does.
        // `--status-level fail` drops the ~2,580 per-test PASS lines (the
        // summary line still prints) -- with them, this gate alone was 340+
        // KB even when everything passed.
        commands: &[
            &["mise", "run", "build:test-components"],
            &["mise", "run", "build:roym"],
            &["cargo", "nextest", "run", "--workspace", "--status-level", "fail"],
        ],
    },
    Gate {
        name: "doctests",
        skip_on_docs_only: true,
        commands: &[&["cargo", "test", "--workspace", "--doc"]],
    },
    Gate { name: "audit", skip_on_docs_only: false, commands: &[&["cargo", "audit"]] },
    Gate {
        name: "deny-licenses",
        skip_on_docs_only: false,
        commands: &[&["cargo", "deny", "check", "licenses"]],
    },
    Gate { name: "e2e", skip_on_docs_only: true, commands: &[&["mise", "run", "test:e2e"]] },
];

/// A passing gate that printed more than this to its log is worth flagging:
/// it will get worse as more gates land, so nudge the quieting effort now.
const SIZE_WARNING_BYTES: u64 = 50 * 1024;

const FAILURE_TAIL_LINES: usize = 60;

struct GateOutcome {
    name: &'static str,
    passed: bool,
    duration: Duration,
    log_path: PathBuf,
    log_bytes: u64,
}

fn known_gate_names() -> Vec<&'static str> {
    GATES.iter().map(|g| g.name).collect()
}

fn parse_skip_args(
    args: impl Iterator<Item = String>,
    known_gates: &[&str],
) -> Result<HashSet<String>> {
    let mut skip = HashSet::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "--skip" {
            let gate = args.next().ok_or_else(|| anyhow!("--skip requires a gate name"))?;
            if !known_gates.contains(&gate.as_str()) {
                bail!("unknown gate `{gate}` for --skip (known gates: {})", known_gates.join(", "));
            }
            skip.insert(gate);
        } else {
            bail!("unknown `xtask verify` argument: {arg} (expected --skip <gate>)");
        }
    }
    Ok(skip)
}

pub(crate) fn is_docs_only_path(path: &str) -> bool {
    path.ends_with(".md") || path.starts_with("docs/")
}

pub(crate) fn is_docs_only_change(paths: &[String]) -> bool {
    !paths.is_empty() && paths.iter().all(|p| is_docs_only_path(p))
}

/// Every path touched relative to `main`, plus anything uncommitted
/// (staged, unstaged, or untracked). Git failures (e.g. no `main` ref, or a
/// plain non-git checkout) are swallowed: an empty result means "not
/// docs-only", which is the safe default -- it runs every gate.
fn changed_paths(workspace_root: &Path) -> Vec<String> {
    let mut paths = HashSet::new();

    if let Ok(out) = Command::new("git")
        .args(["diff", "--name-only", "main..."])
        .current_dir(workspace_root)
        .output()
        && out.status.success()
    {
        paths.extend(String::from_utf8_lossy(&out.stdout).lines().map(str::trim).map(String::from));
    }

    if let Ok(out) =
        Command::new("git").args(["status", "--porcelain"]).current_dir(workspace_root).output()
        && out.status.success()
    {
        paths.extend(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.get(3..))
                .map(str::trim)
                .map(String::from),
        );
    }

    paths.into_iter().filter(|p| !p.is_empty()).collect()
}

pub(crate) fn tail_lines(content: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

pub(crate) fn exceeds_size_warning(log_bytes: u64) -> bool {
    log_bytes > SIZE_WARNING_BYTES
}

/// Runs a gate's commands with stdout/stderr going straight to its log file
/// (the same file handle for both, so real chronological order is kept, the
/// way `cmd >file 2>&1` would) rather than buffered in memory: a long gate's
/// progress is visible on disk as it runs, and a killed process still leaves
/// whatever it had written so far.
fn run_gate(gate: &Gate, log_dir: &Path) -> Result<GateOutcome> {
    let log_path = log_dir.join(format!("{}.log", gate.name));
    let mut log_file = fs::File::create(&log_path)?;
    let start = Instant::now();
    let mut passed = true;

    for argv in gate.commands {
        let (program, cmd_args) = argv
            .split_first()
            .ok_or_else(|| anyhow!("gate `{}` has an empty command", gate.name))?;
        writeln!(log_file, "$ {}", argv.join(" "))?;

        let status = Command::new(program)
            .args(cmd_args)
            .current_dir(crate::get_workspace_root())
            .stdout(log_file.try_clone()?)
            .stderr(log_file.try_clone()?)
            .status()
            .map_err(|e| anyhow!("failed to run `{}`: {e}", argv.join(" ")))?;

        if !status.success() {
            passed = false;
            break;
        }
    }

    let log_bytes = fs::metadata(&log_path)?.len();
    Ok(GateOutcome { name: gate.name, passed, duration: start.elapsed(), log_path, log_bytes })
}

pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let skip = parse_skip_args(args, &known_gate_names())?;
    let workspace_root = crate::get_workspace_root();
    let log_dir = workspace_root.join("target/gates");
    fs::create_dir_all(&log_dir)?;

    let docs_only = is_docs_only_change(&changed_paths(&workspace_root));
    if docs_only {
        println!("Docs-only change detected -- skipping nextest, doctests and e2e.");
    }

    let mut ran = 0;
    let mut any_failed = false;

    for gate in GATES {
        if skip.contains(gate.name) {
            println!("- {} (skipped: --skip)", gate.name);
            continue;
        }
        if docs_only && gate.skip_on_docs_only {
            println!("- {} (skipped: docs-only change)", gate.name);
            continue;
        }

        let outcome = run_gate(gate, &log_dir)?;
        ran += 1;

        if outcome.passed {
            println!("\u{2713} {} ({:.0}s)", outcome.name, outcome.duration.as_secs_f64());
            if exceeds_size_warning(outcome.log_bytes) {
                println!(
                    "\u{26a0} {} printed {} KB on success -- make it quieter",
                    outcome.name,
                    outcome.log_bytes / 1024
                );
            }
        } else {
            any_failed = true;
            println!(
                "\u{2717} {} ({:.0}s) -- see {}",
                outcome.name,
                outcome.duration.as_secs_f64(),
                outcome.log_path.display()
            );
            let content = fs::read_to_string(&outcome.log_path).unwrap_or_default();
            println!("{}", tail_lines(&content, FAILURE_TAIL_LINES));
        }
    }

    println!();
    if any_failed {
        bail!("verify failed: see the gate logs above under target/gates/");
    }
    println!("All {ran} gates passed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_only_change_true_for_markdown_and_docs_dir() {
        let paths = vec!["README.md".to_string(), "docs/planning/deferred-backlog.md".to_string()];
        assert!(is_docs_only_change(&paths));
    }

    #[test]
    fn docs_only_change_false_when_any_path_is_code() {
        let paths = vec!["README.md".to_string(), "xtask/src/verify.rs".to_string()];
        assert!(!is_docs_only_change(&paths));
    }

    #[test]
    fn docs_only_change_false_when_empty() {
        assert!(!is_docs_only_change(&[]));
    }

    #[test]
    fn docs_only_path_accepts_top_level_markdown() {
        assert!(is_docs_only_path("AGENTS.md"));
        assert!(is_docs_only_path("docs/VISION.md"));
        assert!(!is_docs_only_path("crates/router/src/lib.rs"));
    }

    #[test]
    fn tail_lines_keeps_only_the_last_n() {
        let content = (1..=100).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let tail = tail_lines(&content, 3);
        assert_eq!(tail, "line 98\nline 99\nline 100");
    }

    #[test]
    fn tail_lines_returns_everything_when_shorter_than_limit() {
        let content = "a\nb\nc";
        assert_eq!(tail_lines(content, 60), "a\nb\nc");
    }

    #[test]
    fn size_warning_threshold() {
        assert!(!exceeds_size_warning(50 * 1024));
        assert!(exceeds_size_warning(50 * 1024 + 1));
    }

    const FIXTURE_GATES: &[&str] = &["e2e", "audit", "nextest"];

    #[test]
    fn parse_skip_args_collects_repeated_flags() -> Result<()> {
        let args = vec![
            "--skip".to_string(),
            "e2e".to_string(),
            "--skip".to_string(),
            "audit".to_string(),
        ];
        let skip = parse_skip_args(args.into_iter(), FIXTURE_GATES)?;
        assert_eq!(skip, HashSet::from(["e2e".to_string(), "audit".to_string()]));
        Ok(())
    }

    #[test]
    fn parse_skip_args_rejects_unknown_flag() {
        let args = vec!["--bogus".to_string()];
        assert!(parse_skip_args(args.into_iter(), FIXTURE_GATES).is_err());
    }

    #[test]
    fn parse_skip_args_rejects_dangling_skip() {
        let args = vec!["--skip".to_string()];
        assert!(parse_skip_args(args.into_iter(), FIXTURE_GATES).is_err());
    }

    #[test]
    fn parse_skip_args_rejects_unknown_gate_name() {
        // A typo like `--skip nexttest` must fail loudly instead of silently
        // running `nextest` anyway.
        let args = vec!["--skip".to_string(), "nexttest".to_string()];
        assert!(parse_skip_args(args.into_iter(), FIXTURE_GATES).is_err());
    }

    #[test]
    fn known_gate_names_are_unique_and_nonempty() {
        let names = known_gate_names();
        let unique: HashSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len(), "duplicate gate name in GATES");
        assert!(!names.is_empty());
    }
}
