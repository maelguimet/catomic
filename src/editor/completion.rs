//! Purpose: derive local word-completion candidates from caller-bounded text.
//! Owns: scalar-column prefix extraction and deterministic word candidate selection.
//! Must not: access Buffer/App state, scan files, spawn work, allocate an index, or render.
//! Invariants: callers choose the bounded input; candidates are unique and capped.
//! Phase: 5 local current-buffer completion foundation.

use std::collections::BTreeSet;

pub(crate) fn prefix_before_cursor(line: &str, scalar_col: usize) -> String {
    let before: String = line.chars().take(scalar_col).collect();
    let start = before
        .char_indices()
        .rfind(|(_, ch)| !is_word_char(*ch))
        .map_or(0, |(byte, ch)| byte + ch.len_utf8());
    before[start..].to_string()
}

pub(crate) fn complete_words<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    prefix: &str,
    max_candidates: usize,
) -> Vec<String> {
    if prefix.is_empty() || max_candidates == 0 {
        return Vec::new();
    }
    let mut candidates = BTreeSet::new();
    for word in lines
        .into_iter()
        .flat_map(|line| line.split(|ch: char| !is_word_char(ch)))
        .filter(|word| *word != prefix && word.starts_with(prefix))
    {
        candidates.insert(word.to_string());
        if candidates.len() > max_candidates {
            candidates.pop_last();
        }
    }
    candidates.into_iter().collect()
}

pub(crate) fn is_word_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_uses_scalar_column_and_identifier_boundary() {
        assert_eq!(prefix_before_cursor("βeta value_na rest", 13), "value_na");
        assert_eq!(prefix_before_cursor("call(foo", 8), "foo");
        assert_eq!(prefix_before_cursor("word ", 5), "");
    }

    #[test]
    fn candidates_are_unique_sorted_extensions_of_prefix() {
        let lines = ["alpha alpine alpha", "alphabet beta", "al"];

        assert_eq!(
            complete_words(lines.iter().copied(), "al", 8),
            vec!["alpha", "alphabet", "alpine"]
        );
    }

    #[test]
    fn candidate_limit_and_identifier_rules_are_bounded() {
        let lines = ["item_one item-two item3 item_one"];

        assert_eq!(
            complete_words(lines.iter().copied(), "item", 2),
            vec!["item3", "item_one"]
        );
        assert!(complete_words(lines.iter().copied(), "", 8).is_empty());
        assert!(complete_words(lines.iter().copied(), "item", 0).is_empty());
    }
}
