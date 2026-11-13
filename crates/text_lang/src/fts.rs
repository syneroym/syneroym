use crate::search_key::{self, QueryTerms};

#[cfg(test)]
mod tests;

/// The options of the trigram table, as written inside `fts5(...)`.
/// The key is already lower case.
pub const TRI_TABLE_OPTIONS: &str = "tokenize = 'trigram case_sensitive 1'";

/// The options of the word table: tokenizer and prefix index.
pub const WORD_TABLE_OPTIONS: &str =
    "tokenize = \"unicode61 remove_diacritics 0 categories 'L* N* Co M*'\", prefix = '1 2'";

/// Most query words one search may have.
pub const MAX_SEARCH_TERMS: usize = 16;

/// Longest search text, in bytes.
pub const MAX_SEARCH_BYTES: usize = 1024;

/// What the two tables index for one text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexKeys {
    /// For the word table: the key words, one space between them.
    pub words: String,
    /// For the trigram table: the key words with no space.
    pub joined: String,
}

/// Computes the word and trigram index keys for a given text.
pub fn index_keys(text: &str) -> IndexKeys {
    let words = search_key::search_key(text);
    let joined = words.replace(' ', "");
    IndexKeys { words, joined }
}

fn build_term_expr(column: &str, terms: &[String], is_prefix: bool) -> Option<String> {
    if terms.is_empty() {
        return None;
    }

    let star = if is_prefix { "*" } else { "" };
    let clauses: Vec<String> = terms
        .iter()
        .map(|t| {
            let escaped = t.replace('"', "\"\"");
            format!("{column} : \"{escaped}\"{star}")
        })
        .collect();

    Some(clauses.join(" AND "))
}

/// FTS5 MATCH texts for the two tables, for one column. `None` when the
/// query has no term of that kind. Long terms: `col : "t1" AND col : "t2"`;
/// short terms: `col : "t1"* AND ...`. A `"` inside a term is doubled.
pub fn match_exprs(column: &str, terms: &QueryTerms) -> (Option<String>, Option<String>) {
    let long_expr = build_term_expr(column, &terms.long, false);
    let short_expr = build_term_expr(column, &terms.short, true);
    (long_expr, short_expr)
}
