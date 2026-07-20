//! Purpose: present Markdown source as a bounded, readable terminal document.
//! Owns: shared pulldown-cmark interpretation, width-aware layout, and preview text.
//! Must not: read files, emit ANSI, mutate source buffers, or run during ordinary typing.
//! Invariants: conversion is explicit; every produced line is bounded to the reading width.

use std::fmt;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::editor::text_layout;

mod table;

const MAX_READING_WIDTH: usize = 100;
const MAX_SOURCE_BYTES: usize = 10 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenderError {
    OversizedSource,
    TableComplexity,
    OutputExpansion,
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OversizedSource => write!(formatter, "buffer exceeds the 10 MiB preview limit"),
            Self::TableComplexity => {
                write!(formatter, "a table exceeds the bounded preview layout")
            }
            Self::OutputExpansion => write!(
                formatter,
                "rendered document exceeds the 32 MiB preview limit"
            ),
        }
    }
}

/// Render one in-memory Markdown document for a terminal content width.
///
/// The reading column is intentionally capped so wide terminals do not turn prose into
/// eye-tracking punishment. Narrow terminals get reflowed prose and stacked tables.
pub(crate) fn render_with_width(source: &str, width: usize) -> Result<String, RenderError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(RenderError::OversizedSource);
    }
    let width = reading_width(width);
    let mut options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    if source.contains("[^") {
        options.insert(Options::ENABLE_FOOTNOTES);
    }
    let mut renderer = PreviewRenderer::new(width);
    for event in Parser::new_ext(source, options) {
        renderer.event(event);
    }
    let raw = renderer.finish()?;
    wrap_document(&raw, width)
}

pub(crate) fn reading_width(width: usize) -> usize {
    width.clamp(1, MAX_READING_WIDTH)
}

struct PreviewRenderer {
    output: String,
    width: usize,
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    code_block: bool,
    table: Option<table::TableBuilder>,
    links: Vec<LinkTarget>,
    error: Option<RenderError>,
}

struct LinkTarget {
    destination: String,
}

impl PreviewRenderer {
    fn new(width: usize) -> Self {
        Self {
            output: String::new(),
            width,
            lists: Vec::new(),
            quote_depth: 0,
            code_block: false,
            table: None,
            links: Vec::new(),
            error: None,
        }
    }

    fn event(&mut self, event: Event<'_>) {
        if self.error.is_some() {
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => self.push(&format!("`{text}`")),
            Event::SoftBreak => self.push(" "),
            Event::HardBreak => self.line_break(),
            Event::Rule => self.rule(),
            Event::TaskListMarker(done) => self.push(if done { "[x] " } else { "[ ] " }),
            Event::Html(text) | Event::InlineHtml(text) => self.text(&text),
            Event::FootnoteReference(label) => self.push(&format!("[^{label}]")),
            Event::InlineMath(text) => self.push(&format!("${text}$")),
            Event::DisplayMath(text) => self.display_math(&text),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.block_start();
                self.push(heading_marker(level));
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
                self.block_start();
            }
            Tag::CodeBlock(kind) => self.start_code_block(kind),
            Tag::List(first) => self.lists.push(first),
            Tag::Item => self.start_item(),
            Tag::FootnoteDefinition(label) => {
                self.block_start();
                self.push(&format!("[^{label}] "));
            }
            Tag::Table(alignments) => {
                self.block_start();
                self.table = Some(table::TableBuilder::new(alignments));
            }
            Tag::TableHead => self.with_table(|table| table.start_header()),
            Tag::TableRow => self.with_table(|table| table.start_row()),
            Tag::TableCell => self.with_table(|table| table.start_cell()),
            Tag::Emphasis => self.push("*"),
            Tag::Strong => self.push("**"),
            Tag::Strikethrough => self.push("~~"),
            Tag::Link { dest_url, .. } => self.start_link(dest_url.into_string(), false),
            Tag::Image { dest_url, .. } => self.start_link(dest_url.into_string(), true),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.blank_line(),
            TagEnd::Heading(level) => self.end_heading(level),
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.blank_line();
            }
            TagEnd::CodeBlock => self.end_code_block(),
            TagEnd::List(_) => {
                self.lists.pop();
                self.blank_line();
            }
            TagEnd::Item => self.newline(),
            TagEnd::FootnoteDefinition => self.blank_line(),
            TagEnd::TableHead => self.with_table(|table| table.end_header()),
            TagEnd::TableRow => self.with_table(|table| table.end_row()),
            TagEnd::TableCell => self.with_table(|table| table.end_cell()),
            TagEnd::Table => self.end_table(),
            TagEnd::Emphasis => self.push("*"),
            TagEnd::Strong => self.push("**"),
            TagEnd::Strikethrough => self.push("~~"),
            TagEnd::Link | TagEnd::Image => self.end_link(),
            _ => {}
        }
    }

    fn start_item(&mut self) {
        self.block_start();
        self.push(&"  ".repeat(self.lists.len().saturating_sub(1)));
        let marker = match self.lists.last_mut() {
            Some(Some(next)) => {
                let marker = format!("{next}. ");
                *next = next.saturating_add(1);
                marker
            }
            _ => "- ".to_string(),
        };
        self.push(&marker);
    }

    fn end_heading(&mut self, level: HeadingLevel) {
        let line_width = self
            .output
            .rsplit('\n')
            .next()
            .map(|line| text_layout::cell_width_from(line, 0))
            .unwrap_or(0);
        self.newline();
        if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
            let quote_width = self.quote_depth.saturating_mul(2);
            let available = self.width.saturating_sub(quote_width).max(1);
            let underline_width = line_width.saturating_sub(quote_width).min(available).max(1);
            let fill = if level == HeadingLevel::H1 {
                '═'
            } else {
                '─'
            };
            self.push(&fill.to_string().repeat(underline_width));
        }
        self.blank_line();
    }

    fn start_code_block(&mut self, kind: CodeBlockKind<'_>) {
        self.block_start();
        let language = match kind {
            CodeBlockKind::Fenced(language) if !language.is_empty() => {
                let language = language.split_whitespace().next().unwrap_or_default();
                let keep = text_layout::clipped_scalar_len(language, 24);
                language.chars().take(keep).collect::<String>()
            }
            _ => String::new(),
        };
        self.push(&format!("```{language}"));
        self.newline();
        self.code_block = true;
    }

    fn end_code_block(&mut self) {
        self.newline();
        self.code_block = false;
        self.push("```");
        self.blank_line();
    }

    fn start_link(&mut self, destination: String, image: bool) {
        if image {
            self.push("Image: ");
        }
        self.links.push(LinkTarget { destination });
    }

    fn end_link(&mut self) {
        let Some(link) = self.links.pop() else {
            return;
        };
        if !link.destination.is_empty() {
            self.push(&format!(" <{}>", link.destination));
        }
    }

    fn end_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let available = self
            .width
            .saturating_sub(self.quote_depth.saturating_mul(2));
        match table.finish(available.max(1)) {
            Ok(lines) => {
                for line in lines {
                    self.push(&line);
                    self.newline();
                }
            }
            Err(()) => self.error = Some(RenderError::TableComplexity),
        }
        self.blank_line();
    }

    fn rule(&mut self) {
        self.block_start();
        let available = self
            .width
            .saturating_sub(self.quote_depth.saturating_mul(2));
        self.push(&"─".repeat(available.max(1)));
        self.blank_line();
    }

    fn display_math(&mut self, text: &str) {
        self.block_start();
        self.push(text);
        self.blank_line();
    }

    fn text(&mut self, text: &str) {
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                self.line_break();
            }
            if !part.is_empty() {
                self.push(part);
            }
        }
    }

    fn push(&mut self, text: &str) {
        if self.error.is_some() {
            return;
        }
        if let Some(table) = self.table.as_mut() {
            table.push(text);
            return;
        }
        let prefix = self.at_line_start().then(|| self.line_prefix());
        let additional = prefix
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(text.len());
        if self
            .output
            .len()
            .checked_add(additional)
            .is_none_or(|length| length > MAX_OUTPUT_BYTES)
        {
            self.error = Some(RenderError::OutputExpansion);
            return;
        }
        if let Some(prefix) = prefix {
            self.output.push_str(&prefix);
        }
        self.output.push_str(text);
    }

    fn line_prefix(&self) -> String {
        let mut prefix = String::new();
        if self.quote_depth > 0 {
            prefix.push_str(&"> ".repeat(self.quote_depth));
        }
        if self.code_block {
            prefix.push_str("    ");
        }
        prefix
    }

    fn line_break(&mut self) {
        if let Some(table) = self.table.as_mut() {
            table.push_break();
        } else {
            self.newline();
        }
    }

    fn with_table(&mut self, action: impl FnOnce(&mut table::TableBuilder)) {
        if let Some(table) = self.table.as_mut() {
            action(table);
        }
    }

    fn block_start(&mut self) {
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn newline(&mut self) {
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn blank_line(&mut self) {
        self.newline();
        if !self.output.ends_with("\n\n") {
            self.output.push('\n');
        }
    }

    fn at_line_start(&self) -> bool {
        self.output.is_empty() || self.output.ends_with('\n')
    }

    fn finish(mut self) -> Result<String, RenderError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        while self.output.ends_with("\n\n") {
            self.output.pop();
        }
        Ok(self.output)
    }
}

fn heading_marker(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "# ",
        HeadingLevel::H2 => "## ",
        HeadingLevel::H3 => "### ",
        HeadingLevel::H4 => "#### ",
        HeadingLevel::H5 => "##### ",
        HeadingLevel::H6 => "###### ",
    }
}

fn wrap_document(source: &str, width: usize) -> Result<String, RenderError> {
    let mut output = String::with_capacity(source.len());
    for line in source.lines() {
        for wrapped in wrap_line(line, width) {
            if output
                .len()
                .checked_add(wrapped.len().saturating_add(1))
                .is_none_or(|length| length > MAX_OUTPUT_BYTES)
            {
                return Err(RenderError::OutputExpansion);
            }
            output.push_str(&wrapped);
            output.push('\n');
        }
    }
    while output.ends_with("\n\n") {
        output.pop();
    }
    Ok(output)
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let (quote, rest) = quote_prefix(line);
    if rest.chars().next().is_some_and(|ch| "┌│╞└".contains(ch))
        || rest.chars().all(|ch| matches!(ch, '─' | '═'))
    {
        return vec![line.to_string()];
    }

    if let Some(code) = rest.strip_prefix("    ") {
        let prefix = format!("{quote}    ");
        return wrap_prefixed(code, &prefix, &prefix, width, true);
    }
    if rest.starts_with("```") {
        return hard_wrap_line(line, width);
    }

    let indent = rest
        .len()
        .saturating_sub(rest.trim_start_matches(' ').len());
    let after_indent = &rest[indent..];
    if let Some(marker_len) = list_marker_len(after_indent) {
        let marker = &after_indent[..marker_len];
        let first = format!("{quote}{}{marker}", " ".repeat(indent));
        let continuation = format!("{quote}{}", " ".repeat(indent + marker_len));
        return wrap_prefixed(
            &after_indent[marker_len..],
            &first,
            &continuation,
            width,
            false,
        );
    }

    let hashes = after_indent.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && after_indent.as_bytes().get(hashes) == Some(&b' ') {
        let marker_len = hashes + 1;
        let first = format!("{quote}{}", &after_indent[..marker_len]);
        let continuation = format!("{quote}{}", " ".repeat(marker_len));
        return wrap_prefixed(
            &after_indent[marker_len..],
            &first,
            &continuation,
            width,
            false,
        );
    }

    wrap_prefixed(rest, &quote, &quote, width, false)
}

fn quote_prefix(mut line: &str) -> (String, &str) {
    let mut prefix = String::new();
    while let Some(rest) = line.strip_prefix("> ") {
        prefix.push_str("> ");
        line = rest;
    }
    (prefix, line)
}

fn list_marker_len(text: &str) -> Option<usize> {
    if text.starts_with("- ") {
        return Some(2);
    }
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0
        && text.as_bytes().get(digits) == Some(&b'.')
        && text.as_bytes().get(digits + 1) == Some(&b' '))
    .then_some(digits + 2)
}

fn wrap_prefixed(
    content: &str,
    first_prefix: &str,
    continuation_prefix: &str,
    width: usize,
    preserve_spacing: bool,
) -> Vec<String> {
    let first_prefix = fitting_prefix(first_prefix, width);
    let continuation_prefix = fitting_prefix(continuation_prefix, width);
    let first_width = text_layout::cell_width_from(&first_prefix, 0);
    let continuation_width = text_layout::cell_width_from(&continuation_prefix, 0);
    let safe = if preserve_spacing {
        text_layout::expand_tabs(content, false, first_width)
    } else {
        text_layout::terminal_safe_text(content)
    };
    let words: Vec<&str> = if preserve_spacing {
        vec![safe.as_str()]
    } else {
        safe.split_whitespace().collect()
    };
    if words.is_empty() {
        return vec![first_prefix.trim_end().to_string()];
    }

    let mut output = Vec::new();
    let mut prefix = first_prefix;
    let mut prefix_width = first_width;
    let mut line = String::new();
    for word in words {
        let separator = usize::from(!line.is_empty() && !preserve_spacing);
        let available = width.saturating_sub(prefix_width);
        let needed = text_layout::cell_width_from(word, 0).saturating_add(separator);
        if !line.is_empty()
            && needed > available.saturating_sub(text_layout::cell_width_from(&line, 0))
        {
            output.push(format!("{prefix}{line}"));
            prefix = continuation_prefix.clone();
            prefix_width = continuation_width;
            line.clear();
        }
        if !line.is_empty() && !preserve_spacing {
            line.push(' ');
        }
        let mut remaining = word;
        loop {
            let available = width
                .saturating_sub(prefix_width)
                .saturating_sub(text_layout::cell_width_from(&line, 0));
            if text_layout::cell_width_from(remaining, 0) <= available {
                line.push_str(remaining);
                break;
            }
            let (chunk, rest) = split_cells(remaining, available.max(1));
            line.push_str(&chunk);
            output.push(format!("{prefix}{line}"));
            prefix = continuation_prefix.clone();
            prefix_width = continuation_width;
            line.clear();
            remaining = rest;
            if remaining.is_empty() {
                break;
            }
        }
    }
    if !line.is_empty() {
        output.push(format!("{prefix}{line}"));
    }
    output
}

fn fitting_prefix(prefix: &str, width: usize) -> String {
    if text_layout::cell_width_from(prefix, 0) < width {
        prefix.to_string()
    } else {
        String::new()
    }
}

fn hard_wrap_line(line: &str, width: usize) -> Vec<String> {
    let safe = text_layout::terminal_safe_text(line);
    let mut output = Vec::new();
    let mut remaining = safe.as_str();
    while !remaining.is_empty() {
        let (chunk, rest) = split_cells(remaining, width.max(1));
        output.push(chunk);
        remaining = rest;
    }
    output
}

fn split_cells(text: &str, max_cells: usize) -> (String, &str) {
    use unicode_segmentation::UnicodeSegmentation;

    let mut bytes = 0;
    let mut cells = 0;
    for grapheme in text.graphemes(true) {
        let width = text_layout::cell_width_from(grapheme, cells);
        if cells.saturating_add(width) > max_cells {
            break;
        }
        cells = cells.saturating_add(width);
        bytes += grapheme.len();
    }
    if bytes == 0 {
        let first = text.graphemes(true).next().unwrap_or("");
        return ("…".to_string(), &text[first.len()..]);
    }
    (text[..bytes].to_string(), &text[bytes..])
}

#[cfg(test)]
mod tests;
