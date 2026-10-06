//! Check the front matter of change docs under `docs/planning/changes/`.
//!
//! A change doc is the temporary workspace for one feature: spec, design and
//! the list of places where the code differed from the plan. When the work is
//! done, its result moves into the living docs. This check makes that move
//! visible: a `done` change doc must name the living docs it updated (or say
//! why none needed an update), and every named file must exist.
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
const STATUSES: &[&str] = &["draft", "approved", "in-progress", "done", "abandoned"];

#[derive(Debug, PartialEq, Eq)]
enum Touched {
    Path(String),
    None(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct FrontMatter {
    status: Option<String>,
    touched: Vec<Touched>,
}

/// Splits `text` into the front-matter lines and the remaining body.
fn split_front_matter(text: &str) -> Option<(Vec<&str>, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let body_start = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);
    Some((rest[..end].lines().collect(), body_start))
}

fn parse_front_matter(lines: &[&str]) -> FrontMatter {
    let mut fm = FrontMatter::default();
    let mut in_touched = false;
    for line in lines {
        if let Some(item) = line.trim_start().strip_prefix("- ").filter(|_| in_touched) {
            let item = item.trim();
            fm.touched.push(match item.strip_prefix("none:") {
                Some(reason) => Touched::None(reason.trim().to_string()),
                None => Touched::Path(item.to_string()),
            });
        } else if let Some(value) = line.strip_prefix("status:") {
            fm.status = Some(value.trim().to_string());
            in_touched = false;
        } else {
            in_touched = line.trim_end() == "living-docs-touched:";
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
        problems.extend(validate_close_out(&fm.touched, exists));
    }
    problems
}

fn validate_close_out(touched: &[Touched], exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    if touched.is_empty() {
        return vec![
            "status is `done` but `living-docs-touched` is empty; list the living docs you \
             updated, or write `- none: <reason>`"
                .to_string(),
        ];
    }
    touched
        .iter()
        .filter_map(|t| match t {
            Touched::Path(p) if !exists(p) => {
                Some(format!("living-docs-touched names `{p}`, which does not exist"))
            }
            Touched::None(reason) if reason.is_empty() => {
                Some("`- none:` needs a reason after the colon".to_string())
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

    #[test]
    fn done_without_living_docs_is_reported() {
        let problems = validate(&doc("status: done"), &always);
        assert!(problems[0].contains("living-docs-touched"), "{problems:?}");
    }

    #[test]
    fn done_with_existing_path_passes() {
        let front = "status: done\nliving-docs-touched:\n  - docs/a.md";
        assert!(validate(&doc(front), &always).is_empty());
    }

    #[test]
    fn done_with_missing_path_is_reported() {
        let front = "status: done\nliving-docs-touched:\n  - docs/a.md";
        let problems = validate(&doc(front), &never);
        assert!(problems[0].contains("does not exist"), "{problems:?}");
    }

    #[test]
    fn done_with_none_and_reason_passes() {
        let front = "status: done\nliving-docs-touched:\n  - none: internal refactor";
        assert!(validate(&doc(front), &never).is_empty());
    }

    #[test]
    fn none_without_reason_is_reported() {
        let front = "status: done\nliving-docs-touched:\n  - none:";
        assert!(validate(&doc(front), &never)[0].contains("reason"));
    }
}
