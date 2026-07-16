//! Purpose: classify Markdown constructs within one visible logical line.
//! Owns: heading, list/quote marker, fence delimiter, and inline-code spans.
//! Must not: retain fence state, inspect other lines, emit ANSI, or mutate text.
//! Invariants: returned spans are ordered, non-overlapping scalar ranges.
//! Phase: 4-a viewport-only Markdown styling.

use super::{SpanStyle, StyledSpan};

pub(super) fn spans(line: &str) -> Vec<StyledSpan> {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let indent = chars.iter().take_while(|ch| ch.is_whitespace()).count();
    if heading(&chars, indent) || fence(&chars, indent) {
        let style = if chars.get(indent) == Some(&'#') {
            SpanStyle::Heading
        } else {
            SpanStyle::Code
        };
        return vec![StyledSpan {
            start: 0,
            end: len,
            style,
        }];
    }

    let mut spans = marker_end(&chars, indent)
        .map(|end| StyledSpan {
            start: indent,
            end,
            style: SpanStyle::Marker,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < len {
        if chars[index] != '`' {
            index += 1;
            continue;
        }
        let Some(close) = chars[index + 1..].iter().position(|ch| *ch == '`') else {
            break;
        };
        let end = index + close + 2;
        spans.push(StyledSpan {
            start: index,
            end,
            style: SpanStyle::Code,
        });
        index = end;
    }
    spans.sort_by_key(|span| span.start);
    spans
}

fn heading(chars: &[char], indent: usize) -> bool {
    let hashes = chars[indent..].iter().take_while(|ch| **ch == '#').count();
    (1..=6).contains(&hashes)
        && chars
            .get(indent + hashes)
            .is_some_and(|ch| ch.is_whitespace())
}

fn fence(chars: &[char], indent: usize) -> bool {
    let tail = &chars[indent..];
    tail.starts_with(&['`', '`', '`']) || tail.starts_with(&['~', '~', '~'])
}

fn marker_end(chars: &[char], indent: usize) -> Option<usize> {
    let first = *chars.get(indent)?;
    if matches!(first, '>' | '-' | '*' | '+')
        && chars.get(indent + 1).is_some_and(|ch| ch.is_whitespace())
    {
        return Some(indent + 2);
    }
    let digits = chars[indent..]
        .iter()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    (digits > 0
        && chars.get(indent + digits) == Some(&'.')
        && chars
            .get(indent + digits + 1)
            .is_some_and(|ch| ch.is_whitespace()))
    .then_some(indent + digits + 2)
}
