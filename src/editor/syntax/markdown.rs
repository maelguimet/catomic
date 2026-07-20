//! Purpose: classify Markdown constructs within one visible logical line.
//! Owns: scalar-indexed block markers, tables, links, emphasis, and code spans.
//! Must not: retain multiline state, inspect other lines, emit ANSI, or mutate text.
//! Invariants: returned spans are ordered, non-overlapping scalar ranges.
//! Phase: viewport-only Markdown styling, expanded for issue #54.

use super::{SpanStyle, StyledSpan};

mod inline;

pub(super) fn spans(line: &str) -> Vec<StyledSpan> {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|ch| ch.is_whitespace()).count();
    if heading(&chars, indent) || fence(&chars, indent) {
        let style = if chars.get(indent) == Some(&'#') {
            SpanStyle::Heading
        } else {
            SpanStyle::Code
        };
        return vec![span(0, chars.len(), style)];
    }

    let mut spans = Vec::new();
    add_block_markers(&chars, indent, &mut spans);
    if inline::add_source_spans(&chars, &mut spans) {
        add_table_markers(&chars, &mut spans);
    }
    spans.sort_by_key(|span| span.start);
    spans
}

pub(super) fn preview_spans(line: &str) -> Vec<StyledSpan> {
    let chars: Vec<char> = line.chars().collect();
    let content = preview_content_start(&chars);
    if heading(&chars, content) {
        return vec![span(0, chars.len(), SpanStyle::Heading)];
    }
    if is_code_preview_line(&chars, content) {
        return vec![span(0, chars.len(), SpanStyle::Code)];
    }

    let mut spans = Vec::new();
    let indent = chars.iter().take_while(|ch| ch.is_whitespace()).count();
    add_block_markers(&chars, indent, &mut spans);
    add_preview_markers(&chars, &mut spans);
    if inline::add_source_spans(&chars, &mut spans) {
        add_table_markers(&chars, &mut spans);
    }
    inline::add_preview_destination_spans(&chars, &mut spans);
    spans.sort_by_key(|span| span.start);
    spans
}

fn add_block_markers(chars: &[char], indent: usize, spans: &mut Vec<StyledSpan>) {
    if thematic_rule(chars, indent) {
        push_span(spans, indent, chars.len(), SpanStyle::Marker);
        return;
    }
    let mut index = indent;
    while chars.get(index) == Some(&'>') {
        let end = (index + 2).min(chars.len());
        push_span(spans, index, end, SpanStyle::Marker);
        index = end;
        while chars.get(index).is_some_and(|ch| ch.is_whitespace()) {
            index += 1;
        }
    }
    if let Some(end) = list_marker_end(chars, index) {
        push_span(spans, index, end, SpanStyle::Marker);
        if task_marker_end(chars, end).is_some() {
            push_span(spans, end, end + 4, SpanStyle::Marker);
        }
    }
}

fn add_table_markers(chars: &[char], spans: &mut Vec<StyledSpan>) {
    let mut saw_pipe = false;
    for (index, ch) in chars.iter().enumerate() {
        if *ch == '|' && !is_escaped(chars, index) {
            saw_pipe = true;
            push_span(spans, index, index + 1, SpanStyle::Marker);
        }
    }
    if saw_pipe && table_separator(chars) {
        let mut index = 0;
        while index < chars.len() {
            if matches!(chars[index], ':' | '-') {
                let start = index;
                index += 1;
                while index < chars.len() && matches!(chars[index], ':' | '-') {
                    index += 1;
                }
                push_span(spans, start, index, SpanStyle::Marker);
            } else {
                index += 1;
            }
        }
    }
}

fn add_preview_markers(chars: &[char], spans: &mut Vec<StyledSpan>) {
    if !chars.is_empty() && chars.iter().all(|ch| matches!(ch, '─' | '═')) {
        push_span(spans, 0, chars.len(), SpanStyle::Marker);
        return;
    }
    let indent = chars.iter().take_while(|ch| ch.is_whitespace()).count();
    if chars.get(indent).is_some_and(|ch| "•│┌└╞├┏┗".contains(*ch)) {
        let end = if matches!(chars[indent], '┌' | '└' | '╞' | '├' | '┏' | '┗') {
            chars.len()
        } else {
            (indent + 2).min(chars.len())
        };
        push_span(spans, indent, end, SpanStyle::Marker);
    } else if let Some(end) = ordered_marker_end(chars, indent) {
        push_span(spans, indent, end, SpanStyle::Marker);
    }
    for (index, ch) in chars.iter().enumerate() {
        if "│┆┊".contains(*ch) {
            push_span(spans, index, index + 1, SpanStyle::Marker);
        }
    }
}

fn push_span(spans: &mut Vec<StyledSpan>, start: usize, end: usize, style: SpanStyle) {
    if start < end && !overlaps(spans, start, end) {
        spans.push(span(start, end, style));
    }
}

fn overlaps(spans: &[StyledSpan], start: usize, end: usize) -> bool {
    spans
        .iter()
        .any(|span| start < span.end && end > span.start)
}

fn span(start: usize, end: usize, style: SpanStyle) -> StyledSpan {
    StyledSpan { start, end, style }
}

fn heading(chars: &[char], indent: usize) -> bool {
    let hashes = chars[indent..].iter().take_while(|ch| **ch == '#').count();
    (1..=6).contains(&hashes)
        && chars
            .get(indent + hashes)
            .is_none_or(|ch| ch.is_whitespace())
}

fn fence(chars: &[char], indent: usize) -> bool {
    let tail = &chars[indent..];
    tail.starts_with(&['`', '`', '`']) || tail.starts_with(&['~', '~', '~'])
}

fn thematic_rule(chars: &[char], indent: usize) -> bool {
    let marker = chars[indent..]
        .iter()
        .copied()
        .find(|ch| !ch.is_whitespace());
    let Some(marker @ ('-' | '*' | '_')) = marker else {
        return false;
    };
    chars[indent..]
        .iter()
        .all(|ch| ch.is_whitespace() || *ch == marker)
        && chars[indent..].iter().filter(|ch| **ch == marker).count() >= 3
}

fn list_marker_end(chars: &[char], start: usize) -> Option<usize> {
    let first = *chars.get(start)?;
    if matches!(first, '-' | '*' | '+') && chars.get(start + 1).is_some_and(|ch| ch.is_whitespace())
    {
        return Some(start + 2);
    }
    ordered_marker_end(chars, start)
}

fn ordered_marker_end(chars: &[char], start: usize) -> Option<usize> {
    let digits = chars[start..]
        .iter()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    (digits > 0
        && matches!(chars.get(start + digits), Some('.' | ')'))
        && chars
            .get(start + digits + 1)
            .is_some_and(|ch| ch.is_whitespace()))
    .then_some(start + digits + 2)
}

fn task_marker_end(chars: &[char], start: usize) -> Option<usize> {
    (chars.get(start..start + 4).is_some_and(|part| {
        part[0] == '['
            && matches!(part[1], ' ' | 'x' | 'X')
            && part[2] == ']'
            && part[3].is_whitespace()
    }))
    .then_some(start + 4)
}

fn table_separator(chars: &[char]) -> bool {
    let text: String = chars.iter().collect();
    let mut saw_cell = false;
    for cell in text.trim().trim_matches('|').split('|') {
        let cell = cell.trim();
        let dashes = cell.chars().filter(|ch| *ch == '-').count();
        if dashes < 3 || !cell.chars().all(|ch| matches!(ch, ':' | '-')) {
            return false;
        }
        saw_cell = true;
    }
    saw_cell && chars.contains(&'|')
}

fn is_escaped(chars: &[char], index: usize) -> bool {
    chars[..index]
        .iter()
        .rev()
        .take_while(|ch| **ch == '\\')
        .count()
        % 2
        == 1
}

fn preview_content_start(chars: &[char]) -> usize {
    let indent = chars.iter().take_while(|ch| ch.is_whitespace()).count();
    let mut index = if chars.get(indent) == Some(&'>') {
        indent
    } else {
        0
    };
    while chars.get(index) == Some(&'>') {
        index += 1;
        if chars.get(index).is_some_and(|ch| ch.is_whitespace()) {
            index += 1;
        }
    }
    index
}

fn is_code_preview_line(chars: &[char], content: usize) -> bool {
    let tail = &chars[content..];
    tail.starts_with(&['`', '`', '`']) || tail.starts_with(&[' ', ' ', ' ', ' '])
}
