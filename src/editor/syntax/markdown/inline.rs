//! Purpose: classify self-contained inline Markdown constructs on one visible line.
//! Owns: code delimiters, emphasis pairs, source links, and preview link targets.
//! Must not: retain state, inspect adjacent lines, emit ANSI, or allocate document text.
//! Invariants: higher-priority code/link spans prevent overlapping emphasis spans.
//! Phase: issue #54 viewport-only Markdown styling.

use super::{is_escaped, overlaps, push_span};
use crate::editor::syntax::{SpanStyle, StyledSpan};

pub(super) fn add_source_spans(chars: &[char], spans: &mut Vec<StyledSpan>) -> bool {
    let markers = add_code_spans(chars, spans);
    if markers.link {
        add_link_spans(chars, spans);
    }
    if markers.emphasis {
        for delimiter in [&['~', '~'][..], &['*', '*'], &['_', '_'], &['*'], &['_']] {
            add_delimited_spans(chars, spans, delimiter, SpanStyle::Emphasis);
        }
    }
    markers.table
}

pub(super) fn add_preview_destination_spans(chars: &[char], spans: &mut Vec<StyledSpan>) {
    if chars.contains(&'<') {
        add_preview_link_spans(chars, spans);
    }
}

#[derive(Default)]
struct SourceMarkers {
    link: bool,
    emphasis: bool,
    table: bool,
}

fn add_code_spans(chars: &[char], spans: &mut Vec<StyledSpan>) -> SourceMarkers {
    let mut markers = SourceMarkers::default();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '`' || is_escaped(chars, index) {
            markers.link |= chars[index] == '[';
            markers.emphasis |= matches!(chars[index], '~' | '*' | '_');
            markers.table |= chars[index] == '|';
            index += 1;
            continue;
        }
        let run = chars[index..].iter().take_while(|ch| **ch == '`').count();
        let Some(close) = find_run(chars, index + run, '`', run) else {
            index += run;
            continue;
        };
        push_span(spans, index, close + run, SpanStyle::Code);
        index = close + run;
    }
    markers
}

fn add_link_spans(chars: &[char], spans: &mut Vec<StyledSpan>) {
    let mut index = 0;
    while index < chars.len() {
        let open = if chars.get(index..index + 2) == Some(&['!', '[']) {
            index + 1
        } else if chars.get(index) == Some(&'[') {
            index
        } else {
            index += 1;
            continue;
        };
        let Some(label_end) = find_unescaped(chars, open + 1, ']') else {
            break;
        };
        let Some(end) = link_destination_end(chars, label_end + 1) else {
            index = label_end + 1;
            continue;
        };
        push_span(spans, index, end, SpanStyle::Link);
        index = end;
    }
}

fn link_destination_end(chars: &[char], start: usize) -> Option<usize> {
    match chars.get(start)? {
        '(' => find_unescaped(chars, start + 1, ')').map(|end| end + 1),
        '[' => find_unescaped(chars, start + 1, ']').map(|end| end + 1),
        _ => None,
    }
}

fn add_delimited_spans(
    chars: &[char],
    spans: &mut Vec<StyledSpan>,
    delimiter: &[char],
    style: SpanStyle,
) {
    let mut index = 0;
    while index + delimiter.len() <= chars.len() {
        if chars.get(index..index + delimiter.len()) != Some(delimiter)
            || is_escaped(chars, index)
            || overlaps(spans, index, index + delimiter.len())
        {
            index += 1;
            continue;
        }
        let Some(close) = find_delimiter(chars, index + delimiter.len(), delimiter) else {
            break;
        };
        push_span(spans, index, close + delimiter.len(), style);
        index = close + delimiter.len();
    }
}

fn add_preview_link_spans(chars: &[char], spans: &mut Vec<StyledSpan>) {
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '<' {
            index += 1;
            continue;
        }
        let Some(close) = chars[index + 1..].iter().position(|ch| *ch == '>') else {
            break;
        };
        let end = index + close + 2;
        push_span(spans, index, end, SpanStyle::Link);
        index = end;
    }
}

fn find_run(chars: &[char], start: usize, delimiter: char, count: usize) -> Option<usize> {
    (start..chars.len()).find(|index| {
        !is_escaped(chars, *index)
            && (*index == 0 || chars[*index - 1] != delimiter)
            && chars[*index..]
                .iter()
                .take_while(|ch| **ch == delimiter)
                .count()
                == count
    })
}

fn find_delimiter(chars: &[char], start: usize, delimiter: &[char]) -> Option<usize> {
    (start..=chars.len().saturating_sub(delimiter.len())).find(|index| {
        chars.get(*index..*index + delimiter.len()) == Some(delimiter) && !is_escaped(chars, *index)
    })
}

fn find_unescaped(chars: &[char], start: usize, needle: char) -> Option<usize> {
    (start..chars.len()).find(|index| chars[*index] == needle && !is_escaped(chars, *index))
}
