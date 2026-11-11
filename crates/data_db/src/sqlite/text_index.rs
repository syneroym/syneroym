#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const TRI_TOKENIZE: &str = "tokenize = 'trigram case_sensitive 1'";
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const WORD_TOKENIZE: &str =
    "tokenize = \"unicode61 remove_diacritics 0 categories 'L* N* Co M*'\", prefix = '1 2'";

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests;
