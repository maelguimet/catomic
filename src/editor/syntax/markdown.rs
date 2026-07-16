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

pub(super) fn preview_spans(line: &str) -> Vec<StyledSpan> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    if chars.starts_with(&['▌', ' ']) {
        spans.push(StyledSpan {
            start: 0,
            end: chars.len(),
            style: SpanStyle::Heading,
        });
        return spans;
    }
    let indent = chars.iter().take_while(|ch| ch.is_whitespace()).count();
    let marker_end = if chars
        .get(indent)
        .is_some_and(|marker| *marker == '•' || *marker == '│')
    {
        Some(indent + 2)
    } else {
        ordered_preview_marker(&chars, indent)
    };
    if let Some(end) = marker_end {
        spans.push(StyledSpan {
            start: indent,
            end: end.min(chars.len()),
            style: SpanStyle::Marker,
        });
    }
    add_preview_code_spans(&chars, &mut spans);
    spans.sort_by_key(|span| span.start);
    spans
}

fn ordered_preview_marker(chars: &[char], indent: usize) -> Option<usize> {
    let digits = chars[indent..]
        .iter()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    (digits > 0 && chars.get(indent + digits..indent + digits + 2) == Some(&['.', ' ']))
        .then_some(indent + digits + 2)
}

fn add_preview_code_spans(chars: &[char], spans: &mut Vec<StyledSpan>) {
    let mut start = 0;
    while let Some(open) = chars[start..].iter().position(|ch| *ch == '‹') {
        let open = start + open;
        let Some(close) = chars[open + 1..].iter().position(|ch| *ch == '›') else {
            break;
        };
        let end = open + close + 2;
        spans.push(StyledSpan {
            start: open,
            end,
            style: SpanStyle::Code,
        });
        start = end;
    }
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
