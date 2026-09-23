use std::{fs, process::Command};

use anyhow::{Result, bail};

/// Ceiling on clippy::too_many_lines suppressions across the workspace.
///
/// This is a ratchet: it moves down only. The intent is to cap existing
/// suppressions and ratchet down as oversized functions are decomposed.
pub const MAX_TOO_MANY_LINES_SUPPRESSIONS: usize = 40;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedAttribute {
    pub(crate) is_inner: bool,
    pub(crate) body: String,
    pub(crate) line_no: usize,
    pub(crate) end_byte: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum LintAction {
    Expect,
    Allow,
    DenyOrWarn,
    Other,
}

fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
    i += 2;
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_block_comment(bytes: &[u8], mut i: usize, current_line: &mut usize) -> usize {
    i += 2;
    let mut depth = 1;
    while i + 1 < bytes.len() && depth > 0 {
        if bytes[i] == b'\n' {
            *current_line += 1;
        }
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    i
}

fn skip_string_literal(bytes: &[u8], mut i: usize, current_line: &mut usize) -> usize {
    i += 1;
    while i < bytes.len() && bytes[i] != b'"' {
        if bytes[i] == b'\n' {
            *current_line += 1;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'\n' {
                *current_line += 1;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    if i < bytes.len() {
        i += 1;
    }
    i
}

/// Checks if a raw string literal starts at index `i` (which points at 'r' or
/// 'b' in 'br'). If so, skips the raw string and returns the byte index
/// immediately following it.
pub(crate) fn skip_raw_string_if_starts(
    bytes: &[u8],
    i: usize,
    current_line: &mut usize,
) -> Option<usize> {
    let len = bytes.len();
    let r_pos = if bytes[i] == b'r' {
        i
    } else if bytes[i] == b'b' && i + 1 < len && bytes[i + 1] == b'r' {
        i + 1
    } else {
        return None;
    };

    // Cannot be preceded by an identifier character (e.g. `foo_r#"..."`)
    if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        return None;
    }

    let mut p = r_pos + 1;
    let mut hashes = 0;
    while p < len && bytes[p] == b'#' {
        hashes += 1;
        p += 1;
    }

    if p >= len || bytes[p] != b'"' {
        return None;
    }

    p += 1; // Skip opening quote
    while p < len {
        if bytes[p] == b'\n' {
            *current_line += 1;
            p += 1;
            continue;
        }
        if bytes[p] == b'"'
            && p + hashes < len
            && bytes[p + 1..=p + hashes].iter().all(|&c| c == b'#')
        {
            return Some(p + 1 + hashes);
        }
        p += 1;
    }

    Some(len)
}

fn skip_char_literal(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() && bytes[i] != b'\'' && bytes[i] != b'\n' {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    if i < bytes.len() && bytes[i] == b'\'' {
        i += 1;
    }
    i
}

fn is_char_literal_start(bytes: &[u8], i: usize) -> bool {
    let mut j = i + 1;
    if j < bytes.len() && bytes[j] == b'\\' {
        j += 1;
        while j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'\n' && j - i <= 10 {
            j += 1;
        }
    } else if j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'\n' {
        j += 1;
    }
    j < bytes.len() && bytes[j] == b'\''
}

fn parse_attribute_at(
    content: &str,
    i: usize,
    current_line: &mut usize,
) -> Option<(ParsedAttribute, usize)> {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let start_line = *current_line;
    let mut peek = i + 1;
    let mut is_inner = false;
    if peek < len && bytes[peek] == b'!' {
        is_inner = true;
        peek += 1;
    }
    if peek >= len || bytes[peek] != b'[' {
        return None;
    }

    let body_start = peek + 1;
    let mut bracket_depth = 1;
    let mut scan = body_start;

    while scan < len && bracket_depth > 0 {
        if bytes[scan] == b'\n' {
            *current_line += 1;
            scan += 1;
            continue;
        }
        if let Some(next_scan) = skip_raw_string_if_starts(bytes, scan, current_line) {
            scan = next_scan;
            continue;
        }
        if bytes[scan] == b'"' {
            scan = skip_string_literal(bytes, scan, current_line);
            continue;
        }
        if bytes[scan] == b'[' {
            bracket_depth += 1;
        } else if bytes[scan] == b']' {
            bracket_depth -= 1;
        }
        scan += 1;
    }

    if bracket_depth == 0 {
        let body_end = scan - 1;
        let body = content[body_start..body_end].to_string();
        Some((ParsedAttribute { is_inner, body, line_no: start_line, end_byte: scan }, scan))
    } else {
        None
    }
}

pub(crate) fn extract_attributes(content: &str) -> Vec<ParsedAttribute> {
    let mut attrs = Vec::new();
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut current_line = 1;

    while i < len {
        if bytes[i] == b'\n' {
            current_line += 1;
            i += 1;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i, &mut current_line);
            continue;
        }
        if let Some(next_i) = skip_raw_string_if_starts(bytes, i, &mut current_line) {
            i = next_i;
            continue;
        }
        if bytes[i] == b'"' || (bytes[i] == b'b' && i + 1 < len && bytes[i + 1] == b'"') {
            let quote_idx = if bytes[i] == b'b' { i + 1 } else { i };
            i = skip_string_literal(bytes, quote_idx, &mut current_line);
            continue;
        }
        if bytes[i] == b'\'' && is_char_literal_start(bytes, i) {
            i = skip_char_literal(bytes, i);
            continue;
        }
        if bytes[i] == b'b'
            && i + 1 < len
            && bytes[i + 1] == b'\''
            && is_char_literal_start(bytes, i + 1)
        {
            i = skip_char_literal(bytes, i + 1);
            continue;
        }
        if bytes[i] == b'#'
            && let Some((attr, next_i)) = parse_attribute_at(content, i, &mut current_line)
        {
            attrs.push(attr);
            i = next_i;
            continue;
        }
        i += 1;
    }

    attrs
}

fn skip_comments_and_attributes(content: &str, mut i: usize) -> usize {
    let bytes = content.as_bytes();
    let len = bytes.len();

    loop {
        if i >= len {
            return i;
        }
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i + 1 < len && depth > 0 {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if bytes[i] == b'#' && i + 1 < len && bytes[i + 1] == b'[' {
            i += 2;
            let mut depth = 1;
            while i < len && depth > 0 {
                if bytes[i] == b'[' {
                    depth += 1;
                    i += 1;
                } else if bytes[i] == b']' {
                    depth -= 1;
                    i += 1;
                } else if bytes[i] == b'"' {
                    let mut dummy = 0;
                    i = skip_string_literal(bytes, i, &mut dummy);
                } else if let Some(next_i) = skip_raw_string_if_starts(bytes, i, &mut 0) {
                    i = next_i;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        break;
    }
    i
}

fn strip_fn_qualifiers(mut s: &str) -> &str {
    loop {
        if let Some(rest) = s.strip_prefix("pub") {
            let next_char = rest.chars().next();
            if next_char.is_none_or(|c| c.is_whitespace() || c == '(') {
                let rest = rest.trim_start();
                if let Some(rest2) = rest.strip_prefix('(')
                    && let Some(close_idx) = rest2.find(')')
                {
                    s = rest2[close_idx + 1..].trim_start();
                    continue;
                }
                s = rest;
                continue;
            }
        }
        if let Some(rest) = s.strip_prefix("default")
            && rest.chars().next().is_none_or(char::is_whitespace)
        {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("const")
            && rest.chars().next().is_none_or(char::is_whitespace)
        {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("async")
            && rest.chars().next().is_none_or(char::is_whitespace)
        {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("unsafe")
            && rest.chars().next().is_none_or(char::is_whitespace)
        {
            s = rest.trim_start();
            continue;
        }
        if let Some(rest) = s.strip_prefix("extern")
            && rest.chars().next().is_none_or(|c| c.is_whitespace() || c == '"')
        {
            let rest = rest.trim_start();
            if let Some(rest2) = rest.strip_prefix('"')
                && let Some(close_idx) = rest2.find('"')
            {
                s = rest2[close_idx + 1..].trim_start();
                continue;
            }
            s = rest;
            continue;
        }
        break;
    }
    s
}

/// Checks whether the item following the attribute (skipping stacked attributes
/// and qualifiers) is a function (`fn`).
pub(crate) fn next_item_is_fn(content: &str, start_byte: usize) -> bool {
    let next_byte = skip_comments_and_attributes(content, start_byte);
    if next_byte >= content.len() {
        return false;
    }

    let rem = &content[next_byte..];
    let s = strip_fn_qualifiers(rem.trim_start());

    if let Some(rest) = s.strip_prefix("fn") {
        rest.chars().next().is_none_or(|c| c.is_whitespace() || c == '<' || c == '(')
    } else {
        false
    }
}

/// Finds the enclosing action (`expect`, `allow`, `deny`, `warn`) for each
/// occurrence of `target_lint`.
pub(crate) fn find_lint_actions(attr_body: &str, target_lint: &str) -> Vec<LintAction> {
    let bytes = attr_body.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut current_line = 0;
    let mut stack: Vec<String> = Vec::new();
    let mut actions = Vec::new();

    while i < len {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if let Some(next_i) = skip_raw_string_if_starts(bytes, i, &mut current_line) {
            i = next_i;
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_string_literal(bytes, i, &mut current_line);
            continue;
        }
        if bytes[i] == b')' {
            stack.pop();
            i += 1;
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b':')
            {
                i += 1;
            }
            let token = &attr_body[start..i];
            let mut peek = i;
            while peek < len && bytes[peek].is_ascii_whitespace() {
                peek += 1;
            }
            if peek < len && bytes[peek] == b'(' {
                stack.push(token.to_string());
                i = peek + 1;
                continue;
            }

            // Check if token matches target_lint
            let matches =
                token == target_lint || token.strip_prefix("clippy::") == Some(target_lint);
            if matches {
                let enclosing = stack.iter().rev().find(|&s| s != "cfg_attr");
                let action = match enclosing.map(String::as_str) {
                    Some("expect") => LintAction::Expect,
                    Some("allow") => LintAction::Allow,
                    Some("deny") | Some("warn") => LintAction::DenyOrWarn,
                    _ => LintAction::Other,
                };
                actions.push(action);
            }
            continue;
        }
        i += 1;
    }

    actions
}

fn has_expect_reason(attr_body: &str) -> bool {
    let bytes = attr_body.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            let mut dummy = 0;
            i = skip_string_literal(bytes, i, &mut dummy);
            continue;
        }
        if let Some(next_i) = skip_raw_string_if_starts(bytes, i, &mut 0) {
            i = next_i;
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if &attr_body[start..i] == "reason" {
                let mut peek = i;
                while peek < len && bytes[peek].is_ascii_whitespace() {
                    peek += 1;
                }
                if peek < len && bytes[peek] == b'=' {
                    peek += 1;
                    while peek < len && bytes[peek].is_ascii_whitespace() {
                        peek += 1;
                    }
                    if peek < len && (bytes[peek] == b'"' || bytes[peek] == b'r') {
                        return true;
                    }
                }
            }
            continue;
        }
        i += 1;
    }
    false
}

fn check_attribute(
    attr: &ParsedAttribute,
    content: &str,
    rel_path: &str,
    violations: &mut Vec<String>,
) -> usize {
    let pedantic_actions = find_lint_actions(&attr.body, "pedantic");
    for action in pedantic_actions {
        if action == LintAction::Allow || action == LintAction::Expect {
            violations
                .push(format!("{rel_path}:{}: attribute names clippy::pedantic", attr.line_no));
        }
    }

    let warnings_actions = find_lint_actions(&attr.body, "warnings");
    for action in warnings_actions {
        if action == LintAction::Allow || action == LintAction::Expect {
            violations.push(format!("{rel_path}:{}: attribute names warnings", attr.line_no));
        }
    }

    let mut too_many_lines_count = 0;
    let too_many_lines_actions = find_lint_actions(&attr.body, "too_many_lines");
    for action in too_many_lines_actions {
        match action {
            LintAction::DenyOrWarn => {
                // Deny or warn is not a suppression; ignore.
            }
            LintAction::Allow => {
                violations.push(format!(
                    "{rel_path}:{}: #[allow(clippy::too_many_lines)] is forbidden; use \
                     #[expect(clippy::too_many_lines, reason = \"...\")] instead",
                    attr.line_no
                ));
            }
            LintAction::Expect => {
                if attr.is_inner {
                    violations.push(format!(
                        "{rel_path}:{}: inner attribute (#![...]) names clippy::too_many_lines",
                        attr.line_no
                    ));
                } else if !next_item_is_fn(content, attr.end_byte) {
                    violations.push(format!(
                        "{rel_path}:{}: attribute names clippy::too_many_lines but target is not \
                         a function (AGENTS.md requires per-function exemption)",
                        attr.line_no
                    ));
                } else if !has_expect_reason(&attr.body) {
                    violations.push(format!(
                        "{rel_path}:{}: #[expect(clippy::too_many_lines)] is missing `reason = \
                         \"...\"` (AGENTS.md requires an explicit reason)",
                        attr.line_no
                    ));
                } else {
                    too_many_lines_count += 1;
                }
            }
            LintAction::Other => {}
        }
    }

    too_many_lines_count
}

pub fn check_lint_suppressions() -> Result<()> {
    println!(
        "Checking clippy::too_many_lines suppressions (max {MAX_TOO_MANY_LINES_SUPPRESSIONS})..."
    );
    let workspace_root = crate::get_workspace_root();
    let mut violations = Vec::new();

    crate::workspace_lints::check_workspace_lint_config(&workspace_root, &mut violations)?;

    let output = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "*.rs"])
        .current_dir(&workspace_root)
        .output()?;
    let files = String::from_utf8_lossy(&output.stdout);

    let mut total_suppressions = 0;

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

        let attrs = extract_attributes(&content);
        for attr in &attrs {
            total_suppressions += check_attribute(attr, &content, rel_path, &mut violations);
        }
    }

    if !violations.is_empty() {
        eprintln!("\nFound {} lint suppression violation(s):", violations.len());
        for v in &violations {
            eprintln!("  {v}");
        }
        bail!("clippy::too_many_lines suppression check failed");
    }

    println!(
        "Found {total_suppressions} clippy::too_many_lines suppression(s) across tracked files."
    );

    if total_suppressions > MAX_TOO_MANY_LINES_SUPPRESSIONS {
        bail!(
            "clippy::too_many_lines suppressions ({total_suppressions}) exceed the ceiling of \
             {MAX_TOO_MANY_LINES_SUPPRESSIONS}. Reduce function sizes rather than adding \
             suppressions."
        );
    }

    if total_suppressions < MAX_TOO_MANY_LINES_SUPPRESSIONS {
        bail!(
            "clippy::too_many_lines suppressions ({total_suppressions}) are below the ceiling of \
             {MAX_TOO_MANY_LINES_SUPPRESSIONS}. Lower MAX_TOO_MANY_LINES_SUPPRESSIONS in \
             xtask/src/lint_suppressions.rs to match."
        );
    }

    println!(
        "All clippy::too_many_lines suppressions are within the limit ({total_suppressions} <= \
         {MAX_TOO_MANY_LINES_SUPPRESSIONS})."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_string_not_confusing_following_attribute() {
        let code = r##"
const S: &str = r#"say "hi"#;
#[expect(clippy::too_many_lines, reason = "test")]
fn p() {}
"##;
        let attrs = extract_attributes(code);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].body, "expect(clippy::too_many_lines, reason = \"test\")");
        let mut violations = Vec::new();
        let count = check_attribute(&attrs[0], code, "test.rs", &mut violations);
        assert_eq!(count, 1);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_attribute_inside_raw_string_is_ignored() {
        let code = r##"
const S: &str = r#"
#[expect(clippy::too_many_lines, reason = "fake")]
fn inside_raw() {}
"#;
"##;
        let attrs = extract_attributes(code);
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_byte_raw_string_supported() {
        let code = r##"
const B: &[u8] = br#"data "quoted" text"#;
#[expect(clippy::too_many_lines, reason = "real")]
async fn my_func() {}
"##;
        let attrs = extract_attributes(code);
        assert_eq!(attrs.len(), 1);
        let mut violations = Vec::new();
        let count = check_attribute(&attrs[0], code, "test.rs", &mut violations);
        assert_eq!(count, 1);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_allow_form_is_rejected() {
        let code = r#"
#[allow(clippy::too_many_lines)]
fn bad() {}
"#;
        let attrs = extract_attributes(code);
        assert_eq!(attrs.len(), 1);
        let mut violations = Vec::new();
        let count = check_attribute(&attrs[0], code, "test.rs", &mut violations);
        assert_eq!(count, 0);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("#[allow(clippy::too_many_lines)] is forbidden"));
    }

    #[test]
    fn test_deny_form_is_not_suppression() {
        let code = r#"
#![deny(clippy::too_many_lines)]
fn good() {}
"#;
        let attrs = extract_attributes(code);
        assert_eq!(attrs.len(), 1);
        let mut violations = Vec::new();
        let count = check_attribute(&attrs[0], code, "test.rs", &mut violations);
        assert_eq!(count, 0);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_target_item_must_be_fn() {
        let code_impl = r#"
#[expect(clippy::too_many_lines, reason = "impl block")]
impl MyType {
    fn a() {}
    fn b() {}
}
"#;
        let attrs = extract_attributes(code_impl);
        assert_eq!(attrs.len(), 1);
        let mut violations = Vec::new();
        let count = check_attribute(&attrs[0], code_impl, "test.rs", &mut violations);
        assert_eq!(count, 0);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("target is not a function"));

        let code_mod = r#"
#[expect(clippy::too_many_lines, reason = "mod")]
mod tests {
    fn a() {}
}
"#;
        let attrs_mod = extract_attributes(code_mod);
        let mut violations_mod = Vec::new();
        let count_mod = check_attribute(&attrs_mod[0], code_mod, "test.rs", &mut violations_mod);
        assert_eq!(count_mod, 0);
        assert_eq!(violations_mod.len(), 1);
        assert!(violations_mod[0].contains("target is not a function"));
    }

    #[test]
    fn test_stacked_attributes_and_fn_qualifiers() {
        let code = r#"
#[expect(clippy::too_many_lines, reason = "stacked")]
#[tokio::test]
pub(crate) async unsafe extern "C" fn qualified() {}
"#;
        let attrs = extract_attributes(code);
        assert_eq!(attrs.len(), 2);
        let mut violations = Vec::new();
        let count = check_attribute(&attrs[0], code, "test.rs", &mut violations);
        assert_eq!(count, 1);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_cfg_attr_enclosing_action() {
        let actions = find_lint_actions(
            r#"cfg_attr(test, expect(clippy::too_many_lines, reason = "foo"))"#,
            "too_many_lines",
        );
        assert_eq!(actions, vec![LintAction::Expect]);

        let actions_allow =
            find_lint_actions(r#"cfg_attr(test, allow(clippy::too_many_lines))"#, "too_many_lines");
        assert_eq!(actions_allow, vec![LintAction::Allow]);

        let actions_deny =
            find_lint_actions(r#"cfg_attr(test, deny(clippy::too_many_lines))"#, "too_many_lines");
        assert_eq!(actions_deny, vec![LintAction::DenyOrWarn]);
    }

    #[test]
    fn test_warnings_suppression_is_rejected() {
        let code = r#"
#[allow(warnings)]
fn bad() {}
"#;
        let attrs = extract_attributes(code);
        assert_eq!(attrs.len(), 1);
        let mut violations = Vec::new();
        check_attribute(&attrs[0], code, "test.rs", &mut violations);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("attribute names warnings"));

        let code_inner = r#"
#![allow(warnings)]
fn bad2() {}
"#;
        let attrs_inner = extract_attributes(code_inner);
        assert_eq!(attrs_inner.len(), 1);
        let mut violations_inner = Vec::new();
        check_attribute(&attrs_inner[0], code_inner, "test.rs", &mut violations_inner);
        assert_eq!(violations_inner.len(), 1);
        assert!(violations_inner[0].contains("attribute names warnings"));
    }

    #[test]
    fn test_expect_missing_reason_is_rejected() {
        let code = r#"
#[expect(clippy::too_many_lines)]
fn bad() {}
"#;
        let attrs = extract_attributes(code);
        assert_eq!(attrs.len(), 1);
        let mut violations = Vec::new();
        check_attribute(&attrs[0], code, "test.rs", &mut violations);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("missing `reason = \"...\"`"));
    }
}
