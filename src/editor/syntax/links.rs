//! Purpose: identify bounded HTTP(S) destinations in one visible source slice.
//! Owns: scheme matching, terminal-safe destination bounds, and trailing punctuation trimming.
//! Must not: parse whole documents, perform network work, emit ANSI, or mutate source text.
//! Invariants: returned scalar ranges are ordered, non-overlapping, and safe for OSC 8 output.

use std::sync::Arc;

use super::HyperlinkSpan;

const MAX_LINK_BYTES: usize = 4096;

pub(super) fn spans(line: &str) -> Vec<HyperlinkSpan> {
    let mut links = Vec::new();
    let mut search_byte = 0;
    let mut search_scalar = 0;

    while let Some((start, scheme_len)) = next_scheme(line, search_byte) {
        search_scalar += line[search_byte..start].chars().count();
        let token_end = token_end(line, start + scheme_len);
        let end = trim_trailing_punctuation(line, start, token_end);
        let destination = &line[start..end];
        if valid_destination(destination, scheme_len) {
            links.push(HyperlinkSpan {
                start: search_scalar,
                end: search_scalar + destination.chars().count(),
                destination: Arc::from(destination),
            });
        }

        let next = token_end.max(start + scheme_len);
        search_scalar += line[start..next].chars().count();
        search_byte = next;
    }

    links
}

fn next_scheme(line: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    for (offset, _) in line[from..].char_indices() {
        let start = from + offset;
        if !scheme_boundary(line, start) {
            continue;
        }
        let tail = &bytes[start..];
        if tail
            .get(..8)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(b"https://"))
        {
            return Some((start, 8));
        }
        if tail
            .get(..7)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(b"http://"))
        {
            return Some((start, 7));
        }
    }
    None
}

fn scheme_boundary(line: &str, start: usize) -> bool {
    line[..start].chars().next_back().is_none_or(|previous| {
        !previous.is_ascii_alphanumeric() && !matches!(previous, '+' | '-' | '.')
    })
}

fn token_end(line: &str, after_scheme: usize) -> usize {
    line[after_scheme..]
        .char_indices()
        .find_map(|(offset, ch)| {
            (ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>' | '"' | '\'' | '`'))
                .then_some(after_scheme + offset)
        })
        .unwrap_or(line.len())
}

fn trim_trailing_punctuation(line: &str, start: usize, mut end: usize) -> usize {
    loop {
        let destination = &line[start..end];
        let Some(last) = destination.chars().next_back() else {
            return end;
        };
        let trim = matches!(last, '.' | ',' | ';' | ':' | '!' | '?')
            || unmatched_closer(destination, last);
        if !trim {
            return end;
        }
        end -= last.len_utf8();
    }
}

fn unmatched_closer(destination: &str, closer: char) -> bool {
    let opener = match closer {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        _ => return false,
    };
    destination.chars().filter(|ch| *ch == closer).count()
        > destination.chars().filter(|ch| *ch == opener).count()
}

fn valid_destination(destination: &str, scheme_len: usize) -> bool {
    if destination.len() > MAX_LINK_BYTES
        || destination
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\u{1b}' | '\u{7}'))
    {
        return false;
    }
    destination[scheme_len..]
        .chars()
        .next()
        .is_some_and(|first| !matches!(first, '/' | '?' | '#' | '.' | ',' | ';' | ':'))
}
