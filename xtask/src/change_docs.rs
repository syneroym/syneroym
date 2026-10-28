//! Check the front matter of change docs under `docs/planning/changes/`.
//!
//! A change doc is the temporary workspace for one feature: spec, design and
//! the list of places where the code differed from the plan. When the work is
//! done, its result moves into the living docs. This check makes that move
//! visible: a `done` change doc must name the living docs it updated (or say
//! why none needed an update), and every named file must exist. The same holds
//! for `demo:`, which names the demo stories the change adds or extends (or
//! says why it has none).
//!
//! Front matter format (a subset of YAML, parsed by hand to avoid a new
//! dependency):
//!
//! ```text
//! ---
//! status: done
//! living-docs-touched:
//!   - docs/architecture/router.md
//!   - none: <reason>
//! demo:
//!   - crates/substrate/tests/e2e/demo/10-substrate-main.spec.ts
//! ---
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

const CHANGES_DIR: &str = "docs/planning/changes";
const CHANGE_FILE: &str = "change.md";
const DEVIATIONS_HEADING: &str = "## Deviations";
/// Where demo stories live. A `demo:` path must point into this folder.
const DEMO_DIR: &str = "crates/substrate/tests/e2e/demo/";
const STATUSES: &[&str] = &["draft", "approved", "in-progress", "done", "abandoned"];

/// One item of a front-matter list: a repo-relative path, or `none: <reason>`.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Path(String),
    None(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct FrontMatter {
    status: Option<String>,
    touched: Vec<Entry>,
    demo: Vec<Entry>,
}

/// Splits `text` into the front-matter lines and the remaining body.
fn split_front_matter(text: &str) -> Option<(Vec<&str>, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let body_start = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);
    Some((rest[..end].lines().collect(), body_start))
}

fn parse_entry(item: &str) -> Entry {
    let item = item.trim();
    match item.strip_prefix("none:") {
        Some(reason) => Entry::None(reason.trim().to_string()),
        None => Entry::Path(item.to_string()),
    }
}

fn parse_front_matter(lines: &[&str]) -> FrontMatter {
    #[derive(Clone, Copy, PartialEq)]
    enum List {
        Touched,
        Demo,
    }
    let mut fm = FrontMatter::default();
    let mut current: Option<List> = None;
    for line in lines {
        let item = line.trim_start().strip_prefix("- ");
        match (item, current) {
            (Some(item), Some(List::Touched)) => fm.touched.push(parse_entry(item)),
            (Some(item), Some(List::Demo)) => fm.demo.push(parse_entry(item)),
            _ => {
                if let Some(value) = line.strip_prefix("status:") {
                    fm.status = Some(value.trim().to_string());
                }
                current = match line.trim_end() {
                    "living-docs-touched:" => Some(List::Touched),
                    "demo:" => Some(List::Demo),
                    _ => None,
                };
            }
        }
    }
    fm
}

/// Returns the problems found in one change doc. `exists` reports whether a
/// repo-relative path is present, so tests do not need a real checkout.
fn validate(text: &str, exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    let Some((lines, body)) = split_front_matter(text) else {
        return vec!["missing front matter block (`---` ... `---`)".to_string()];
    };
    let fm = parse_front_matter(&lines);
    let mut problems = Vec::new();

    match fm.status.as_deref() {
        None => problems.push("front matter has no `status:`".to_string()),
        Some(s) if !STATUSES.contains(&s) => {
            problems.push(format!("status `{s}` is not one of {}", STATUSES.join(", ")));
        }
        Some(_) => {}
    }
    if !body.lines().any(|l| l.trim_end() == DEVIATIONS_HEADING) {
        problems.push(format!("missing `{DEVIATIONS_HEADING}` section"));
    }
    if fm.status.as_deref() == Some("done") {
        problems.extend(validate_list(
            "living-docs-touched",
            "the living docs you updated",
            &fm.touched,
            exists,
        ));
        problems.extend(validate_list(
            "demo",
            "the demo stories you added or extended",
            &fm.demo,
            exists,
        ));
        problems.extend(fm.demo.iter().filter_map(demo_path_problem));
    }
    problems
}

/// A `demo:` path must be a story file inside the demo folder, not any file
/// that happens to exist.
fn demo_path_problem(entry: &Entry) -> Option<String> {
    let Entry::Path(p) = entry else { return None };
    let is_story = p.ends_with(".spec.ts") || p.ends_with(".sh");
    let stays_inside = !p.split('/').any(|part| part == "..");
    if p.starts_with(DEMO_DIR) && is_story && stays_inside {
        return None;
    }
    Some(format!(
        "demo names `{p}`, which is not a story: use a `.spec.ts` or `.sh` file under `{DEMO_DIR}`"
    ))
}

/// Checks one close-out list of a `done` doc: it must not be empty, every path
/// must exist, and every `none:` must give a reason.
fn validate_list(
    field: &str,
    what: &str,
    entries: &[Entry],
    exists: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    if entries.is_empty() {
        return vec![format!(
            "status is `done` but `{field}` is empty; list {what}, or write `- none: <reason>`"
        )];
    }
    entries
        .iter()
        .filter_map(|e| match e {
            Entry::Path(p) if !exists(p) => {
                Some(format!("{field} names `{p}`, which does not exist"))
            }
            Entry::None(reason) if reason.is_empty() => {
                Some(format!("`- none:` in `{field}` needs a reason after the colon"))
            }
            _ => None,
        })
        .collect()
}

fn change_doc_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join(CHANGES_DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.is_dir() {
            paths.push(path.join(CHANGE_FILE));
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn check_change_docs() -> Result<()> {
    println!("Checking change doc front matter under {CHANGES_DIR}/...");
    let root = crate::get_workspace_root();
    let exists = |rel: &str| root.join(rel).is_file();
    let mut failures = 0;
    let paths = change_doc_paths(&root)?;

    for path in &paths {
        let shown = path.strip_prefix(&root).unwrap_or(path).display();
        let problems = match fs::read_to_string(path) {
            Ok(text) => validate(&text, &exists),
            Err(_) => vec![format!("each folder needs a `{CHANGE_FILE}` file")],
        };
        for p in &problems {
            eprintln!("  ERROR: {shown}: {p}");
        }
        failures += problems.len();
    }

    if failures > 0 {
        bail!("Change doc check failed with {failures} problem(s)");
    }
    println!("All {} change doc(s) are valid.", paths.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: &str) -> bool {
        true
    }

    fn never(_: &str) -> bool {
        false
    }

    fn doc(front: &str) -> String {
        format!("---\n{front}\n---\n# Title\n\n## Deviations\n\nNone yet.\n")
    }

    #[test]
    fn draft_doc_needs_only_status_and_deviations() {
        assert!(validate(&doc("status: draft"), &always).is_empty());
    }

    #[test]
    fn missing_front_matter_is_reported() {
        assert_eq!(validate("# Title\n", &always).len(), 1);
    }

    #[test]
    fn unknown_status_is_reported() {
        let problems = validate(&doc("status: finished"), &always);
        assert!(problems[0].contains("not one of"), "{problems:?}");
    }

    #[test]
    fn missing_deviations_section_is_reported() {
        let text = "---\nstatus: draft\n---\n# Title\n";
        assert!(validate(text, &always)[0].contains("Deviations"));
    }

    const DEMO_NONE: &str = "demo:\n  - none: internal";

    #[test]
    fn done_without_living_docs_is_reported() {
        let front = format!("status: done\n{DEMO_NONE}");
        let problems = validate(&doc(&front), &always);
        assert!(problems[0].contains("living-docs-touched"), "{problems:?}");
    }

    #[test]
    fn done_with_existing_path_passes() {
        let front = format!("status: done\nliving-docs-touched:\n  - docs/a.md\n{DEMO_NONE}");
        let front = front.as_str();
        assert!(validate(&doc(front), &always).is_empty());
    }

    #[test]
    fn done_with_missing_path_is_reported() {
        let front = format!("status: done\nliving-docs-touched:\n  - docs/a.md\n{DEMO_NONE}");
        let problems = validate(&doc(&front), &never);
        assert!(problems[0].contains("does not exist"), "{problems:?}");
    }

    #[test]
    fn done_with_none_and_reason_passes() {
        let front =
            format!("status: done\nliving-docs-touched:\n  - none: internal refactor\n{DEMO_NONE}");
        assert!(validate(&doc(&front), &never).is_empty());
    }

    #[test]
    fn none_without_reason_is_reported() {
        let front = format!("status: done\nliving-docs-touched:\n  - none:\n{DEMO_NONE}");
        assert!(validate(&doc(&front), &never)[0].contains("reason"));
    }

    const TOUCHED_NONE: &str = "living-docs-touched:\n  - none: internal";

    #[test]
    fn done_without_demo_is_reported() {
        let front = format!("status: done\n{TOUCHED_NONE}");
        let problems = validate(&doc(&front), &always);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("`demo` is empty"), "{problems:?}");
    }

    #[test]
    fn done_with_existing_demo_path_passes() {
        let front = format!(
            "status: done\n{TOUCHED_NONE}\ndemo:\n  - crates/substrate/tests/e2e/demo/10-a.spec.ts"
        );
        assert!(validate(&doc(&front), &always).is_empty());
    }

    #[test]
    fn done_with_missing_demo_path_is_reported() {
        let front = format!(
            "status: done\n{TOUCHED_NONE}\ndemo:\n  - crates/substrate/tests/e2e/demo/10-a.spec.ts"
        );
        let problems = validate(&doc(&front), &never);
        assert!(problems[0].contains("demo names"), "{problems:?}");
    }

    #[test]
    fn demo_path_must_be_a_story_inside_the_demo_folder() {
        let bad = [
            "AGENTS.md",
            "/etc/passwd",
            "crates/substrate/tests/e2e/demo/../global-setup.ts",
            "crates/substrate/tests/e2e/demo/lib.ts",
            "crates/substrate/tests/e2e/tests/webrtc.spec.ts",
        ];
        for path in bad {
            let front = format!("status: done\n{TOUCHED_NONE}\ndemo:\n  - {path}");
            let problems = validate(&doc(&front), &always);
            assert!(problems.iter().any(|p| p.contains("not a story")), "{path}: {problems:?}");
        }
    }

    #[test]
    fn demo_shell_script_is_a_story() {
        let path = "crates/substrate/tests/e2e/demo/10-a.sh";
        let front = format!("status: done\n{TOUCHED_NONE}\ndemo:\n  - {path}");
        assert!(validate(&doc(&front), &always).is_empty());
    }

    #[test]
    fn demo_none_without_reason_is_reported() {
        let front = format!("status: done\n{TOUCHED_NONE}\ndemo:\n  - none:");
        assert!(validate(&doc(&front), &never)[0].contains("`demo`"));
    }

    #[test]
    fn list_items_go_to_the_list_that_precedes_them() {
        let lines = ["status: done", "demo:", "  - a", "living-docs-touched:", "  - b"];
        let fm = parse_front_matter(&lines);
        assert_eq!(fm.demo, vec![Entry::Path("a".into())]);
        assert_eq!(fm.touched, vec![Entry::Path("b".into())]);
    }

    #[test]
    fn draft_doc_may_leave_demo_empty() {
        assert!(validate(&doc("status: draft\ndemo:"), &never).is_empty());
    }
}
