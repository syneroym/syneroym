use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    process::Command,
};

use anyhow::{Result, anyhow, bail};

/// Maximum allowed production source lines (excluding `#[cfg(test)]` blocks).
const MAX_PRODUCTION_LINES: usize = 800;

/// Standard limit for test files and inline test blocks.
const STANDARD_TEST_LIMIT: usize = 800;

/// Maximum allowed test file lines for grandfathered oversized test files.
///
/// This is a ratchet: it moves down only. The intent is to bring it toward the
/// 800-line production limit over time as large test files are decomposed.
const MAX_TEST_LINES: usize = 1800;

fn is_test_path(path: &Path) -> bool {
    for component in path.iter() {
        if component == "tests" {
            return true;
        }
    }
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str())
        && (file_name.starts_with("tests_")
            || file_name.ends_with("_tests.rs")
            || file_name == "tests.rs")
    {
        return true;
    }
    false
}

pub(crate) fn is_test_file(path: &Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some("build.rs") {
        return false;
    }
    let mut in_src = false;
    for component in path.iter() {
        if component == "src" {
            in_src = true;
            break;
        }
    }
    if !in_src {
        return true;
    }
    is_test_path(path)
}

fn skip_comment_in_sanitizer(bytes: &[u8], mut i: usize, out: &mut String) -> Option<usize> {
    let len = bytes.len();
    if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
        out.push(' ');
        out.push(' ');
        i += 2;
        while i < len && bytes[i] != b'\n' {
            out.push(' ');
            i += 1;
        }
        return Some(i);
    }
    if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
        out.push(' ');
        out.push(' ');
        i += 2;
        let mut depth = 1;
        while i < len && depth > 0 {
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                depth += 1;
                out.push(' ');
                out.push(' ');
                i += 2;
            } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                depth -= 1;
                out.push(' ');
                out.push(' ');
                i += 2;
            } else {
                if bytes[i] == b'\n' {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
                i += 1;
            }
        }
        return Some(i);
    }
    None
}

fn skip_string_in_sanitizer(bytes: &[u8], i: usize, out: &mut String) -> Option<usize> {
    let len = bytes.len();
    // Raw string: r#"..."#, br#"..."#, cr#"..."#
    let (is_raw, start_r) =
        if bytes[i] == b'r' && i + 1 < len && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            (true, i)
        } else if (bytes[i] == b'b' || bytes[i] == b'c')
            && i + 2 < len
            && bytes[i + 1] == b'r'
            && (bytes[i + 2] == b'"' || bytes[i + 2] == b'#')
        {
            (true, i + 1)
        } else {
            (false, 0)
        };

    if is_raw {
        let mut hashes = 0;
        let mut j = start_r + 1;
        while j < len && bytes[j] == b'#' {
            hashes += 1;
            j += 1;
        }
        if j < len && bytes[j] == b'"' {
            let quote_pos = j;
            let mut k = quote_pos + 1;
            while k < len {
                if bytes[k] == b'"' {
                    let mut match_hashes = true;
                    for h in 0..hashes {
                        if k + 1 + h >= len || bytes[k + 1 + h] != b'#' {
                            match_hashes = false;
                            break;
                        }
                    }
                    if match_hashes {
                        for &b in &bytes[i..=k + hashes] {
                            if b == b'\n' {
                                out.push('\n');
                            } else {
                                out.push(' ');
                            }
                        }
                        return Some(k + hashes + 1);
                    }
                }
                k += 1;
            }
        }
    }

    // Normal string or byte string: "..." or b"..."
    let is_str = bytes[i] == b'"' || (bytes[i] == b'b' && i + 1 < len && bytes[i + 1] == b'"');
    if is_str {
        let str_start = if bytes[i] == b'"' { i } else { i + 1 };
        let mut j = str_start + 1;
        let mut escaped = false;
        while j < len {
            if escaped {
                escaped = false;
            } else if bytes[j] == b'\\' {
                escaped = true;
            } else if bytes[j] == b'"' {
                for &b in &bytes[i..=j] {
                    if b == b'\n' {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                }
                return Some(j + 1);
            }
            j += 1;
        }
    }

    // Char literal: 'x' or '\n'
    if bytes[i] == b'\'' {
        if i + 2 < len && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
            out.push(' ');
            out.push(' ');
            out.push(' ');
            return Some(i + 3);
        }
        if i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'' {
            out.push(' ');
            out.push(' ');
            out.push(' ');
            out.push(' ');
            return Some(i + 4);
        }
    }

    None
}

pub(crate) fn sanitize_code(content: &str) -> String {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        if let Some(next_i) = skip_comment_in_sanitizer(bytes, i, &mut out) {
            i = next_i;
            continue;
        }
        if let Some(next_i) = skip_string_in_sanitizer(bytes, i, &mut out) {
            i = next_i;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

pub(crate) fn count_production_lines(content: &str) -> usize {
    let sanitized = sanitize_code(content);
    let mut prod_lines = 0;
    let mut in_cfg_test = false;
    let mut brace_depth: usize = 0;
    let mut opened_brace = false;

    for line in sanitized.lines() {
        let trimmed = line.trim();

        if !in_cfg_test {
            if trimmed.contains("#[cfg(test)]") {
                in_cfg_test = true;
                opened_brace = false;
                brace_depth = 0;

                let after_attr = if let Some(idx) = line.find("#[cfg(test)]") {
                    &line[idx + "#[cfg(test)]".len()..]
                } else {
                    line
                };

                for c in after_attr.chars() {
                    if c == '{' {
                        opened_brace = true;
                        brace_depth += 1;
                    } else if c == '}' {
                        brace_depth = brace_depth.saturating_sub(1);
                        if brace_depth == 0 && opened_brace {
                            in_cfg_test = false;
                        }
                    } else if c == ';' && !opened_brace {
                        in_cfg_test = false;
                    }
                }
                continue;
            }

            prod_lines += 1;
        } else {
            for c in trimmed.chars() {
                if c == '{' {
                    opened_brace = true;
                    brace_depth += 1;
                } else if c == '}' {
                    brace_depth = brace_depth.saturating_sub(1);
                    if brace_depth == 0 && opened_brace {
                        in_cfg_test = false;
                    }
                } else if c == ';' && !opened_brace {
                    in_cfg_test = false;
                }
            }
        }
    }

    prod_lines
}

fn load_oversized_test_files(workspace_root: &Path) -> Result<BTreeMap<String, usize>> {
    let list_path = workspace_root.join("xtask/oversized-test-files.txt");
    if !list_path.exists() {
        return Ok(BTreeMap::new());
    }
    let content = fs::read_to_string(&list_path)?;
    let mut files = BTreeMap::new();
    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 2 {
            let path = parts[0].trim_end_matches(':');
            let max_lines: usize = parts[1].parse().map_err(|e| {
                anyhow!(
                    "invalid line limit '{}' at {}:{}: {e}",
                    parts[1],
                    list_path.display(),
                    line_no + 1
                )
            })?;
            files.insert(path.to_string(), max_lines);
        } else {
            bail!(
                "invalid format at {}:{}: expected '<path> <max_lines>'",
                list_path.display(),
                line_no + 1
            );
        }
    }
    Ok(files)
}

fn check_test_file_length(
    rel_path: &str,
    total_lines: usize,
    oversized_files: &BTreeMap<String, usize>,
    seen_oversized: &mut HashSet<String>,
    violations: &mut Vec<String>,
) {
    if let Some(&recorded_limit) = oversized_files.get(rel_path) {
        seen_oversized.insert(rel_path.to_string());
        if total_lines <= STANDARD_TEST_LIMIT {
            violations.push(format!(
                "{rel_path}: {total_lines} lines is <= {STANDARD_TEST_LIMIT}; remove from \
                 xtask/oversized-test-files.txt"
            ));
        } else if total_lines > recorded_limit {
            violations.push(format!(
                "{rel_path}: {total_lines} lines exceeds recorded limit of {recorded_limit} in \
                 xtask/oversized-test-files.txt"
            ));
        } else if total_lines > MAX_TEST_LINES {
            violations.push(format!(
                "{rel_path}: {total_lines} lines (maximum allowed for oversized test files is \
                 {MAX_TEST_LINES})"
            ));
        }
    } else if total_lines > STANDARD_TEST_LIMIT {
        violations.push(format!(
            "{rel_path}: {total_lines} lines (maximum allowed for test files is \
             {STANDARD_TEST_LIMIT}; add to xtask/oversized-test-files.txt or decompose)"
        ));
    }
}

fn check_production_file_length(
    rel_path: &str,
    content: &str,
    oversized_files: &BTreeMap<String, usize>,
    seen_oversized: &mut HashSet<String>,
    violations: &mut Vec<String>,
) {
    let total_lines = content.lines().count();
    let prod_lines = count_production_lines(content);
    if prod_lines > MAX_PRODUCTION_LINES {
        violations.push(format!(
            "{rel_path}: {prod_lines} production lines (maximum allowed is {MAX_PRODUCTION_LINES})"
        ));
    }

    let inline_test_lines = total_lines.saturating_sub(prod_lines);
    if let Some(&recorded_limit) = oversized_files.get(rel_path) {
        seen_oversized.insert(rel_path.to_string());
        if inline_test_lines <= STANDARD_TEST_LIMIT {
            violations.push(format!(
                "{rel_path}: inline test lines {inline_test_lines} <= {STANDARD_TEST_LIMIT}; \
                 remove from xtask/oversized-test-files.txt"
            ));
        } else if inline_test_lines > recorded_limit {
            violations.push(format!(
                "{rel_path}: {inline_test_lines} inline test lines exceeds recorded limit of \
                 {recorded_limit} in xtask/oversized-test-files.txt"
            ));
        } else if inline_test_lines > MAX_TEST_LINES {
            violations.push(format!(
                "{rel_path}: {inline_test_lines} inline test lines (maximum allowed for oversized \
                 test blocks is {MAX_TEST_LINES})"
            ));
        }
    } else if inline_test_lines > STANDARD_TEST_LIMIT {
        violations.push(format!(
            "{rel_path}: {inline_test_lines} inline test lines (maximum allowed for inline tests \
             is {STANDARD_TEST_LIMIT}; add to xtask/oversized-test-files.txt or decompose)"
        ));
    }
}

pub fn check_file_lengths() -> Result<()> {
    println!("Checking source file lengths...");
    let workspace_root = crate::get_workspace_root();
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*.rs"])
        .current_dir(&workspace_root)
        .output()
        .map_err(|e| anyhow!("failed to run `git ls-files`: {e}"))?;

    if !output.status.success() {
        bail!("`git ls-files` failed");
    }

    let files = String::from_utf8_lossy(&output.stdout);
    let oversized_files = load_oversized_test_files(&workspace_root)?;
    let mut seen_oversized = HashSet::new();
    let mut violations = Vec::new();
    let mut prod_checked = 0;
    let mut test_checked = 0;

    for line in files.lines() {
        let rel_path = line.trim();
        if rel_path.is_empty() {
            continue;
        }

        let full_path = workspace_root.join(rel_path);
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let path = Path::new(rel_path);
        if is_test_file(path) {
            test_checked += 1;
            let total_lines = content.lines().count();
            check_test_file_length(
                rel_path,
                total_lines,
                &oversized_files,
                &mut seen_oversized,
                &mut violations,
            );
        } else {
            prod_checked += 1;
            check_production_file_length(
                rel_path,
                &content,
                &oversized_files,
                &mut seen_oversized,
                &mut violations,
            );
        }
    }

    for listed in oversized_files.keys() {
        if !seen_oversized.contains(listed) {
            violations.push(format!(
                "{listed}: listed in xtask/oversized-test-files.txt but does not exist in tracked \
                 files"
            ));
        }
    }

    if !violations.is_empty() {
        for v in &violations {
            eprintln!("ERROR: {v}");
        }
        bail!("File length check failed with {} violation(s)", violations.len());
    }

    println!(
        "All {prod_checked} production files (<= {MAX_PRODUCTION_LINES} lines) and {test_checked} \
         test files (<= {STANDARD_TEST_LIMIT} lines, or within recorded limits in \
         xtask/oversized-test-files.txt) adhere to length limits."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_production_lines_basic() {
        let code = "pub fn foo() {\n    println!(\"hello\");\n}\n";
        assert_eq!(count_production_lines(code), 3);
    }

    #[test]
    fn test_count_production_lines_one_line_mod_tests() {
        let code = r#"
pub fn foo() {}

#[cfg(test)]
mod tests;

pub fn bar() {
    let x = 1;
}
"#;
        assert_eq!(count_production_lines(code.trim()), 6);
    }

    #[test]
    fn test_count_production_lines_block_tests() {
        let code = r#"
pub fn foo() {}

#[cfg(test)]
mod tests {
    #[test]
    fn t1() {
        assert!(true);
    }
}

pub fn bar() {}
"#;
        assert_eq!(count_production_lines(code.trim()), 4);
    }

    #[test]
    fn test_count_production_lines_ignores_cfg_test_in_comments_and_strings() {
        let code = r##"
/// Doc comment mentioning #[cfg(test)] blocks
pub fn foo() {
    let s = "#[cfg(test)] mod tests;";
    let r = r#"#[cfg(test)]"#;
}
"##;
        assert_eq!(count_production_lines(code.trim()), 5);
    }

    #[test]
    fn test_count_production_lines_cfg_test_use() {
        let code = r#"
pub fn foo() {}

#[cfg(test)]
use some_crate::tests::*;

pub fn bar() {}
"#;
        assert_eq!(count_production_lines(code.trim()), 4);
    }

    #[test]
    fn test_count_production_lines_single_line_attr_and_mod() {
        let code = r#"
pub fn foo() {}
#[cfg(test)] mod tests;
pub fn bar() {}
"#;
        assert_eq!(count_production_lines(code.trim()), 2);
    }

    #[test]
    fn test_load_oversized_test_files_requires_number() -> Result<()> {
        let temp_dir = std::env::temp_dir().join(format!("xtask_test_{}", std::process::id()));
        let xtask_dir = temp_dir.join("xtask");
        fs::create_dir_all(&xtask_dir)?;
        let list_file = xtask_dir.join("oversized-test-files.txt");

        // Valid
        fs::write(&list_file, "crates/foo/tests/bar.rs 900\n")?;
        let map = load_oversized_test_files(&temp_dir)?;
        assert_eq!(map.get("crates/foo/tests/bar.rs"), Some(&900));

        // Missing number should fail
        fs::write(&list_file, "crates/foo/tests/bar.rs\n")?;
        assert!(load_oversized_test_files(&temp_dir).is_err());

        let _ = fs::remove_dir_all(&temp_dir);
        Ok(())
    }
}
