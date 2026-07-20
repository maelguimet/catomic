//! Purpose: extract colon-prefixed emoji queries and rank bundled emoji metadata.
//! Owns: token boundaries, deterministic fuzzy matching, and bounded candidate results.
//! Must not: access App/buffers, render, mutate text, perform I/O, or contact a network.
//! Invariants: matching is case-insensitive; ranking and result bounds are deterministic.

use std::cmp::Ordering;

pub(crate) const MAX_QUERY_SCALARS: usize = 32;
pub(crate) const MIN_QUERY_SCALARS: usize = 2;
pub(crate) const MAX_RESULTS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmojiQuery {
    pub(crate) colon_col: usize,
    pub(crate) text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct EmojiCandidate {
    pub(crate) glyph: &'static str,
    pub(crate) name: &'static str,
    pub(crate) aliases: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MatchRank {
    class: u8,
    position: usize,
    gaps: usize,
}

struct RankedEmoji {
    rank: MatchRank,
    emoji: &'static emojis::Emoji,
}

/// Extract a query ending at `cursor_col` from a caller-bounded line window.
/// `window_start_col` is the absolute scalar column of `window` in the document line.
pub(crate) fn query_before_cursor(
    window: &str,
    window_start_col: usize,
    cursor_col: usize,
) -> Option<EmojiQuery> {
    let chars: Vec<char> = window.chars().collect();
    let relative_cursor = cursor_col.checked_sub(window_start_col)?;
    if relative_cursor > chars.len() {
        return None;
    }
    let mut query_start = relative_cursor;
    while query_start > 0 && is_query_char(chars[query_start - 1]) {
        query_start -= 1;
    }
    let colon = query_start.checked_sub(1)?;
    if chars[colon] != ':' || (colon > 0 && !is_token_boundary(chars[colon - 1])) {
        return None;
    }
    let query: String = chars[query_start..relative_cursor].iter().collect();
    let query_len = query.chars().count();
    if !(MIN_QUERY_SCALARS..=MAX_QUERY_SCALARS).contains(&query_len) {
        return None;
    }
    Some(EmojiQuery {
        colon_col: window_start_col.saturating_add(colon),
        text: query,
    })
}

pub(crate) fn ranked_candidates(query: &str) -> Vec<EmojiCandidate> {
    let query = normalize(query);
    if query.is_empty() {
        return Vec::new();
    }
    let mut best = Vec::<RankedEmoji>::with_capacity(MAX_RESULTS);
    for emoji in emojis::iter() {
        let name_rank = match_rank(&normalize(emoji.name()), &query)
            .map(|rank| rank.with_alias_priority(false));
        let alias_rank = emoji
            .shortcodes()
            .filter_map(|alias| {
                match_rank(&normalize(alias), &query).map(|rank| rank.with_alias_priority(true))
            })
            .min();
        let Some(rank) = name_rank.into_iter().chain(alias_rank).min() else {
            continue;
        };
        insert_ranked(&mut best, RankedEmoji { rank, emoji });
    }
    best.into_iter()
        .map(|ranked| EmojiCandidate {
            glyph: ranked.emoji.as_str(),
            name: ranked.emoji.name(),
            aliases: ranked.emoji.shortcodes().take(3).collect(),
        })
        .collect()
}

impl MatchRank {
    fn with_alias_priority(mut self, alias: bool) -> Self {
        if self.class > 0 {
            self.class = self
                .class
                .saturating_mul(2)
                .saturating_sub(u8::from(!alias));
        }
        self
    }
}

fn insert_ranked(best: &mut Vec<RankedEmoji>, candidate: RankedEmoji) {
    let position = best
        .binary_search_by(|existing| compare_ranked(existing, &candidate))
        .unwrap_or_else(|position| position);
    best.insert(position, candidate);
    best.truncate(MAX_RESULTS);
}

fn compare_ranked(left: &RankedEmoji, right: &RankedEmoji) -> Ordering {
    left.rank
        .cmp(&right.rank)
        .then_with(|| left.emoji.name().cmp(right.emoji.name()))
        .then_with(|| left.emoji.as_str().cmp(right.emoji.as_str()))
}

fn match_rank(candidate: &str, query: &str) -> Option<MatchRank> {
    if candidate == query {
        return Some(MatchRank {
            class: 0,
            position: 0,
            gaps: 0,
        });
    }
    if candidate.starts_with(query) {
        return Some(MatchRank {
            class: 1,
            position: 0,
            gaps: candidate.len().saturating_sub(query.len()),
        });
    }
    if let Some(position) = word_prefix_position(candidate, query) {
        return Some(MatchRank {
            class: 2,
            position,
            gaps: candidate.len().saturating_sub(query.len()),
        });
    }
    if let Some(position) = candidate.find(query) {
        return Some(MatchRank {
            class: 3,
            position,
            gaps: candidate.len().saturating_sub(query.len()),
        });
    }
    subsequence_rank(candidate, query)
}

fn word_prefix_position(candidate: &str, query: &str) -> Option<usize> {
    candidate
        .match_indices(' ')
        .map(|(position, _)| position.saturating_add(1))
        .find(|&position| candidate[position..].starts_with(query))
}

fn subsequence_rank(candidate: &str, query: &str) -> Option<MatchRank> {
    let mut query_chars = query.chars();
    let mut wanted = query_chars.next()?;
    let mut first = None;
    for (position, character) in candidate.char_indices() {
        if character != wanted {
            continue;
        }
        first.get_or_insert(position);
        let Some(next) = query_chars.next() else {
            let start = first.unwrap_or(position);
            return Some(MatchRank {
                class: 4,
                position: start,
                gaps: position
                    .saturating_sub(start)
                    .saturating_add(1)
                    .saturating_sub(query.len()),
            });
        };
        wanted = next;
    }
    None
}

fn normalize(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_space = false;
    for character in text.chars() {
        let character = if character == '_' || character == '-' || character.is_whitespace() {
            ' '
        } else {
            character.to_ascii_lowercase()
        };
        if character == ' ' {
            if previous_space {
                continue;
            }
            previous_space = true;
        } else {
            previous_space = false;
        }
        normalized.push(character);
    }
    normalized.trim().to_string()
}

fn is_query_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '+')
}

fn is_token_boundary(character: char) -> bool {
    !character.is_alphanumeric() && !matches!(character, '_' | ':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_query_at_start_and_after_token_boundary() {
        assert_eq!(
            query_before_cursor(":hun", 0, 4),
            Some(EmojiQuery {
                colon_col: 0,
                text: "hun".to_string()
            })
        );
        assert_eq!(
            query_before_cursor("say (:ROCK", 0, 10),
            Some(EmojiQuery {
                colon_col: 5,
                text: "ROCK".to_string()
            })
        );
    }

    #[test]
    fn rejects_short_queries_and_colons_inside_tokens() {
        assert!(query_before_cursor(":h", 0, 2).is_none());
        assert!(query_before_cursor("word:hun", 0, 8).is_none());
        assert!(query_before_cursor("12:30", 0, 5).is_none());
        assert!(query_before_cursor("::hun", 0, 5).is_none());
    }

    #[test]
    fn bounded_window_returns_absolute_colon_column() {
        assert_eq!(
            query_before_cursor(" :hun", 40, 45),
            Some(EmojiQuery {
                colon_col: 41,
                text: "hun".to_string()
            })
        );
    }

    #[test]
    fn hundred_points_is_first_for_hun() {
        let matches = ranked_candidates("HuN");
        assert_eq!(matches.first().map(|candidate| candidate.glyph), Some("💯"));
        assert_eq!(
            matches.first().map(|candidate| candidate.name),
            Some("hundred points")
        );
    }

    #[test]
    fn exact_alias_outranks_fuzzy_matches() {
        let matches = ranked_candidates("100");
        assert_eq!(matches.first().map(|candidate| candidate.glyph), Some("💯"));
        assert!(matches[0].aliases.contains(&"100"));
    }

    #[test]
    fn fuzzy_subsequence_matching_is_bounded_and_deterministic() {
        let first = ranked_candidates("rckt")
            .into_iter()
            .map(|candidate| candidate.glyph)
            .collect::<Vec<_>>();
        let second = ranked_candidates("rckt")
            .into_iter()
            .map(|candidate| candidate.glyph)
            .collect::<Vec<_>>();
        assert!(!first.is_empty());
        assert!(first.len() <= MAX_RESULTS);
        assert_eq!(first, second);
    }
}
