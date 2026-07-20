//! Purpose: present Markdown source as a bounded, readable terminal document.
//! Owns: shared pulldown-cmark interpretation, width-aware layout, and semantic styling.
//! Must not: read files, emit ANSI, mutate source buffers, or run during ordinary typing.
//! Invariants: conversion is explicit; every produced line is bounded to the reading width.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;

use crate::editor::syntax::{HyperlinkSpan, SpanStyle, StyledSpan};
use crate::editor::text_layout;

mod table;

const MAX_READING_WIDTH: usize = 88;
const MIN_MARGIN_WIDTH: usize = 40;
const DOCUMENT_MARGIN: usize = 2;
const MAX_SOURCE_BYTES: usize = 10 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_ANNOTATIONS: usize = 1_000_000;
const MAX_LINK_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkdownDocument {
    pub(crate) text: String,
    pub(crate) spans: Vec<Vec<StyledSpan>>,
    pub(crate) links: Vec<Vec<HyperlinkSpan>>,
}

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
pub(crate) fn render_with_width(
    source: &str,
    width: usize,
) -> Result<MarkdownDocument, RenderError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(RenderError::OversizedSource);
    }
    let layout_width = layout_width(width);
    let (width, margin) = document_layout(layout_width);
    let mut options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    if source.contains("[^") {
        options.insert(Options::ENABLE_FOOTNOTES);
    }
    let mut renderer = PreviewRenderer::new(width);
    for event in Parser::new_ext(source, options) {
        renderer.event(event);
    }
    wrap_document(renderer.finish()?, width, margin)
}

pub(crate) fn layout_width(width: usize) -> usize {
    width.max(1)
}

fn document_layout(width: usize) -> (usize, usize) {
    let minimum_margin = if width >= MIN_MARGIN_WIDTH {
        DOCUMENT_MARGIN
    } else {
        0
    };
    let reading_width = width
        .saturating_sub(minimum_margin.saturating_mul(2))
        .clamp(1, MAX_READING_WIDTH);
    (reading_width, width.saturating_sub(reading_width) / 2)
}

struct PreviewRenderer {
    output: String,
    scalar_len: usize,
    spans: Vec<GlobalSpan>,
    hyperlinks: Vec<GlobalHyperlink>,
    active_styles: Vec<SpanStyle>,
    width: usize,
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    code_block: Option<String>,
    table: Option<table::TableBuilder>,
    links: Vec<LinkTarget>,
    error: Option<RenderError>,
}

#[derive(Clone, Copy)]
struct GlobalSpan {
    start: usize,
    end: usize,
    style: SpanStyle,
}

struct GlobalHyperlink {
    start: usize,
    end: usize,
    destination: Arc<str>,
}

struct RawDocument {
    text: String,
    spans: Vec<GlobalSpan>,
    hyperlinks: Vec<GlobalHyperlink>,
}

struct LinkTarget {
    start: usize,
    destination: Option<Arc<str>>,
}

impl PreviewRenderer {
    fn new(width: usize) -> Self {
        Self {
            output: String::new(),
            scalar_len: 0,
            spans: Vec::new(),
            hyperlinks: Vec::new(),
            active_styles: Vec::new(),
            width,
            lists: Vec::new(),
            quote_depth: 0,
            code_block: None,
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
            Event::Code(text) => self.push_styled(&safe_text(&text), SpanStyle::PreviewInlineCode),
            Event::SoftBreak => self.push(" "),
            Event::HardBreak => self.line_break(),
            Event::Rule => self.rule(),
            Event::TaskListMarker(done) => {
                self.push_styled(if done { "[✓] " } else { "[ ] " }, SpanStyle::Marker)
            }
            Event::Html(text) | Event::InlineHtml(text) => self.text(&text),
            Event::FootnoteReference(label) => {
                self.push_styled(&format!("[^{label}]"), SpanStyle::Marker)
            }
            Event::InlineMath(text) => {
                self.push_styled(&safe_text(&text), SpanStyle::PreviewInlineCode)
            }
            Event::DisplayMath(text) => self.display_math(&text),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => {
                self.start_heading(level);
                self.active_styles.push(heading_style(level));
                let indent = heading_indent(level);
                if indent > 0 {
                    self.push(&" ".repeat(indent));
                }
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
                self.push_styled(&format!("[^{label}] "), SpanStyle::Marker);
            }
            Tag::Table(alignments) => {
                self.block_start();
                self.table = Some(table::TableBuilder::new(alignments));
            }
            Tag::TableHead => self.with_table(|table| table.start_header()),
            Tag::TableRow => self.with_table(|table| table.start_row()),
            Tag::TableCell => self.with_table(|table| table.start_cell()),
            Tag::Emphasis => self.active_styles.push(SpanStyle::PreviewEmphasis),
            Tag::Strong => self.active_styles.push(SpanStyle::PreviewStrong),
            Tag::Strikethrough => self.active_styles.push(SpanStyle::PreviewStrikethrough),
            Tag::Link { dest_url, .. } => self.start_link(dest_url.into_string(), false),
            Tag::Image { dest_url, .. } => self.start_link(dest_url.into_string(), true),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_paragraph(),
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
            TagEnd::Emphasis => self.end_style(SpanStyle::PreviewEmphasis),
            TagEnd::Strong => self.end_style(SpanStyle::PreviewStrong),
            TagEnd::Strikethrough => self.end_style(SpanStyle::PreviewStrikethrough),
            TagEnd::Link | TagEnd::Image => self.end_link(),
            _ => {}
        }
    }

    fn start_heading(&mut self, level: HeadingLevel) {
        if self.output.is_empty() {
            return;
        }
        if matches!(
            level,
            HeadingLevel::H1 | HeadingLevel::H2 | HeadingLevel::H3
        ) {
            self.blank_line();
        } else {
            self.newline();
        }
    }

    fn start_paragraph(&mut self) {
        if self.quote_depth == 0 {
            return;
        }
        self.push_styled("“", SpanStyle::Marker);
    }

    fn end_paragraph(&mut self) {
        if self.quote_depth > 0 {
            self.push_styled("”", SpanStyle::Marker);
        }
        self.blank_line();
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
            _ => "• ".to_string(),
        };
        self.push_styled(&marker, SpanStyle::Marker);
    }

    fn end_heading(&mut self, level: HeadingLevel) {
        self.end_style(heading_style(level));
        self.newline();
        if matches!(level, HeadingLevel::H1 | HeadingLevel::H2) {
            self.blank_line();
        }
    }

    fn start_code_block(&mut self, _kind: CodeBlockKind<'_>) {
        self.block_start();
        self.code_block = Some(String::new());
    }

    fn end_code_block(&mut self) {
        let code = self.code_block.take().unwrap_or_default();
        self.render_code_block(&code);
        self.blank_line();
    }

    fn render_code_block(&mut self, code: &str) {
        let prefix = self.quote_prefix();
        let prefix_width = text_layout::cell_width_from(&prefix, 0);
        let available = self.width.saturating_sub(prefix_width).max(1);
        let padding: usize = if available >= 6 {
            4
        } else if available >= 3 {
            1
        } else {
            0
        };
        let content_width = available.saturating_sub(padding).max(1);
        let code = code.strip_suffix('\n').unwrap_or(code);
        for source_line in code.split('\n') {
            let safe_line = text_layout::expand_tabs(source_line, false, 0);
            for line in wrap_code_line(&safe_line, content_width) {
                if !prefix.is_empty() {
                    self.append(&prefix, Some(SpanStyle::Marker));
                }
                let block = format!("{}{}", " ".repeat(padding), line);
                self.append(&block, Some(SpanStyle::PreviewCodeBlock));
                self.append("\n", None);
            }
        }
    }

    fn start_link(&mut self, destination: String, image: bool) {
        if image {
            self.push_styled("Image: ", SpanStyle::Marker);
        }
        self.links.push(LinkTarget {
            start: self.scalar_len,
            destination: safe_link_destination(destination),
        });
        self.active_styles.push(SpanStyle::PreviewLink);
    }

    fn end_link(&mut self) {
        self.end_style(SpanStyle::PreviewLink);
        let Some(link) = self.links.pop() else {
            return;
        };
        if let Some(destination) = link.destination {
            if link.start < self.scalar_len {
                self.hyperlinks.push(GlobalHyperlink {
                    start: link.start,
                    end: self.scalar_len,
                    destination,
                });
            }
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
                    let blank = line.style == table::TableLineStyle::Blank;
                    self.push_table_line(line);
                    if blank {
                        self.blank_line();
                    } else {
                        self.newline();
                    }
                }
            }
            Err(()) => self.error = Some(RenderError::TableComplexity),
        }
        self.blank_line();
    }

    fn push_table_line(&mut self, line: table::TableLine) {
        self.push(&line.text);
        let line_end = self.scalar_len;
        let line_start = line_end.saturating_sub(line.text.chars().count());
        match line.style {
            table::TableLineStyle::Header => {
                self.add_span(line_start, line_end, SpanStyle::PreviewStrong)
            }
            table::TableLineStyle::Label => {
                let label_len = line
                    .text
                    .split_once(':')
                    .map_or(0, |(label, _)| label.len() + 1);
                let label_scalars = line.text[..label_len].chars().count();
                self.add_span(
                    line_start,
                    line_start.saturating_add(label_scalars),
                    SpanStyle::PreviewStrong,
                );
            }
            table::TableLineStyle::Body | table::TableLineStyle::Blank => {}
        }
    }

    fn rule(&mut self) {
        self.block_start();
        let marker = "·  ·  ·";
        let available = self
            .width
            .saturating_sub(text_layout::cell_width_from(&self.quote_prefix(), 0));
        let marker = if available < text_layout::cell_width_from(marker, 0) {
            "···"
        } else {
            marker
        };
        let padding = available.saturating_sub(text_layout::cell_width_from(marker, 0)) / 2;
        self.push(&" ".repeat(padding));
        self.push_styled(marker, SpanStyle::Marker);
        self.blank_line();
    }

    fn display_math(&mut self, text: &str) {
        self.block_start();
        self.render_code_block(text);
        self.blank_line();
    }

    fn text(&mut self, text: &str) {
        if let Some(code) = self.code_block.as_mut() {
            code.push_str(text);
            return;
        }
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                self.line_break();
            }
            if !part.is_empty() {
                self.push(&safe_text(part));
            }
        }
    }

    fn push(&mut self, text: &str) {
        self.push_with_style(text, None);
    }

    fn push_styled(&mut self, text: &str, style: SpanStyle) {
        self.push_with_style(text, Some(style));
    }

    fn push_with_style(&mut self, text: &str, style: Option<SpanStyle>) {
        if self.error.is_some() {
            return;
        }
        if let Some(code) = self.code_block.as_mut() {
            code.push_str(text);
            return;
        }
        if let Some(table) = self.table.as_mut() {
            table.push(text);
            return;
        }
        if self.at_line_start() {
            let prefix = self.line_prefix();
            if !prefix.is_empty() {
                self.append(&prefix, Some(SpanStyle::Marker));
            }
        }
        self.append(text, style);
    }

    fn append(&mut self, text: &str, extra_style: Option<SpanStyle>) {
        if self.error.is_some() || text.is_empty() {
            return;
        }
        if self
            .output
            .len()
            .checked_add(text.len())
            .is_none_or(|length| length > MAX_OUTPUT_BYTES)
        {
            self.error = Some(RenderError::OutputExpansion);
            return;
        }
        let start = self.scalar_len;
        self.output.push_str(text);
        self.scalar_len = self.scalar_len.saturating_add(text.chars().count());
        let end = self.scalar_len;
        for index in 0..self.active_styles.len() {
            let style = self.active_styles[index];
            self.add_span(start, end, style);
        }
        if let Some(style) = extra_style {
            self.add_span(start, end, style);
        }
    }

    fn add_span(&mut self, start: usize, end: usize, style: SpanStyle) {
        if start >= end {
            return;
        }
        if self.spans.len() >= MAX_ANNOTATIONS {
            self.error = Some(RenderError::OutputExpansion);
            return;
        }
        self.spans.push(GlobalSpan { start, end, style });
    }

    fn line_prefix(&self) -> String {
        self.quote_prefix()
    }

    fn quote_prefix(&self) -> String {
        "  ".repeat(self.quote_depth)
    }

    fn line_break(&mut self) {
        if let Some(code) = self.code_block.as_mut() {
            if !code.ends_with('\n') {
                code.push('\n');
            }
        } else if let Some(table) = self.table.as_mut() {
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
            self.append("\n", None);
        }
    }

    fn newline(&mut self) {
        if !self.output.ends_with('\n') {
            self.append("\n", None);
        }
    }

    fn blank_line(&mut self) {
        self.newline();
        if !self.output.ends_with("\n\n") {
            self.append("\n", None);
        }
    }

    fn at_line_start(&self) -> bool {
        self.output.is_empty() || self.output.ends_with('\n')
    }

    fn end_style(&mut self, style: SpanStyle) {
        if let Some(index) = self
            .active_styles
            .iter()
            .rposition(|active| *active == style)
        {
            self.active_styles.remove(index);
        }
    }

    fn finish(mut self) -> Result<RawDocument, RenderError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        while self.output.ends_with("\n\n") {
            self.output.pop();
            self.scalar_len = self.scalar_len.saturating_sub(1);
        }
        self.spans.retain(|span| span.start < self.scalar_len);
        for span in &mut self.spans {
            span.end = span.end.min(self.scalar_len);
        }
        self.hyperlinks
            .retain(|link| link.start < self.scalar_len && link.start < link.end);
        for link in &mut self.hyperlinks {
            link.end = link.end.min(self.scalar_len);
        }
        Ok(RawDocument {
            text: self.output,
            spans: self.spans,
            hyperlinks: self.hyperlinks,
        })
    }
}

fn heading_style(level: HeadingLevel) -> SpanStyle {
    match level {
        HeadingLevel::H1 => SpanStyle::PreviewHeading1,
        HeadingLevel::H2 => SpanStyle::PreviewHeading2,
        HeadingLevel::H3 => SpanStyle::PreviewHeading3,
        HeadingLevel::H4 => SpanStyle::PreviewHeading4,
        HeadingLevel::H5 => SpanStyle::PreviewHeading5,
        HeadingLevel::H6 => SpanStyle::PreviewHeading6,
    }
}

fn heading_indent(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 2,
        HeadingLevel::H2 => 0,
        HeadingLevel::H3 => 2,
        HeadingLevel::H4 | HeadingLevel::H5 => 4,
        HeadingLevel::H6 => 6,
    }
}

fn wrap_code_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut cells = 0usize;
    for grapheme in line.graphemes(true) {
        let grapheme_width = text_layout::cell_width_from(grapheme, cells);
        if !current.is_empty() && cells.saturating_add(grapheme_width) > width {
            lines.push(current);
            current = String::new();
            cells = 0;
        }
        if current.is_empty() && grapheme_width > width {
            lines.push("…".to_string());
            continue;
        }
        current.push_str(grapheme);
        cells = cells.saturating_add(text_layout::cell_width_from(grapheme, cells));
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn safe_text(text: &str) -> Cow<'_, str> {
    if text.chars().any(char::is_control) {
        Cow::Owned(text_layout::terminal_safe_text(text))
    } else {
        Cow::Borrowed(text)
    }
}

fn safe_link_destination(destination: String) -> Option<Arc<str>> {
    (destination.len() <= MAX_LINK_BYTES
        && !destination
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\u{1b}' | '\u{7}')))
    .then(|| Arc::from(destination))
}

#[derive(Clone)]
struct AnnotatedLine {
    text: String,
    chars: Vec<char>,
    scalar_len: usize,
    spans: Vec<StyledSpan>,
    links: Vec<HyperlinkSpan>,
}

impl AnnotatedLine {
    fn empty() -> Self {
        Self {
            text: String::new(),
            chars: Vec::new(),
            scalar_len: 0,
            spans: Vec::new(),
            links: Vec::new(),
        }
    }

    fn len(&self) -> usize {
        self.scalar_len
    }

    fn slice_text(&self, start: usize, end: usize) -> String {
        self.chars[start.min(self.len())..end.min(self.len())]
            .iter()
            .collect()
    }
}

struct LineBuilder {
    text: String,
    len: usize,
    spans: Vec<StyledSpan>,
    links: Vec<HyperlinkSpan>,
}

impl LineBuilder {
    fn new() -> Self {
        Self {
            text: String::new(),
            len: 0,
            spans: Vec::new(),
            links: Vec::new(),
        }
    }

    fn push_slice(&mut self, source: &AnnotatedLine, start: usize, end: usize) {
        let start = start.min(source.len());
        let end = end.min(source.len());
        if start >= end {
            return;
        }
        let target = self.len;
        self.text.push_str(&source.slice_text(start, end));
        self.len += end - start;
        for span in &source.spans {
            let overlap_start = span.start.max(start);
            let overlap_end = span.end.min(end);
            if overlap_start < overlap_end {
                self.spans.push(StyledSpan {
                    start: target + overlap_start - start,
                    end: target + overlap_end - start,
                    style: span.style,
                });
            }
        }
        for link in &source.links {
            let overlap_start = link.start.max(start);
            let overlap_end = link.end.min(end);
            if overlap_start < overlap_end {
                self.links.push(HyperlinkSpan {
                    start: target + overlap_start - start,
                    end: target + overlap_end - start,
                    destination: link.destination.clone(),
                });
            }
        }
    }

    fn push_plain(&mut self, text: &str) {
        self.text.push_str(text);
        self.len += text.chars().count();
    }

    fn push_replacement(&mut self, source: &AnnotatedLine, col: usize, text: &str) {
        let start = self.len;
        self.push_plain(text);
        let end = self.len;
        for span in source
            .spans
            .iter()
            .filter(|span| col >= span.start && col < span.end)
        {
            self.spans.push(StyledSpan {
                start,
                end,
                style: span.style,
            });
        }
        if let Some(link) = source
            .links
            .iter()
            .find(|link| col >= link.start && col < link.end)
        {
            self.links.push(HyperlinkSpan {
                start,
                end,
                destination: link.destination.clone(),
            });
        }
    }

    fn width(&self) -> usize {
        text_layout::cell_width_from(&self.text, 0)
    }

    fn trim_end(&mut self) {
        while self.text.ends_with(' ') {
            self.text.pop();
            self.len = self.len.saturating_sub(1);
        }
        self.spans.retain(|span| span.start < self.len);
        for span in &mut self.spans {
            span.end = span.end.min(self.len);
        }
        self.links.retain(|link| link.start < self.len);
        for link in &mut self.links {
            link.end = link.end.min(self.len);
        }
    }

    fn finish(mut self) -> AnnotatedLine {
        self.trim_end();
        AnnotatedLine {
            text: self.text,
            chars: Vec::new(),
            scalar_len: self.len,
            spans: self.spans,
            links: self.links,
        }
    }
}

fn wrap_document(
    raw: RawDocument,
    width: usize,
    margin: usize,
) -> Result<MarkdownDocument, RenderError> {
    if raw.text.is_empty() {
        return Ok(MarkdownDocument {
            text: String::new(),
            spans: Vec::new(),
            links: Vec::new(),
        });
    }
    let mut lines = Vec::new();
    let mut line_starts = Vec::new();
    let mut global_start = 0usize;
    for text in raw.text.split('\n') {
        let len = text.chars().count();
        line_starts.push(global_start);
        lines.push(AnnotatedLine {
            text: text.to_string(),
            chars: Vec::new(),
            scalar_len: len,
            spans: Vec::new(),
            links: Vec::new(),
        });
        global_start = global_start.saturating_add(len).saturating_add(1);
    }
    distribute_spans(&mut lines, &line_starts, &raw.spans);
    distribute_links(&mut lines, &line_starts, &raw.hyperlinks);
    let lines = lines
        .into_iter()
        .flat_map(|line| wrap_line(line, width))
        .map(|line| fill_heading_band(line, width))
        .collect::<Vec<_>>();

    let mut text = String::with_capacity(raw.text.len());
    let mut spans = Vec::with_capacity(lines.len());
    let mut links = Vec::with_capacity(lines.len());
    for mut line in lines {
        if margin > 0 && !line.text.is_empty() {
            line.text.insert_str(0, &" ".repeat(margin));
            line.scalar_len = line.scalar_len.saturating_add(margin);
            for span in &mut line.spans {
                span.start = span.start.saturating_add(margin);
                span.end = span.end.saturating_add(margin);
            }
            for link in &mut line.links {
                link.start = link.start.saturating_add(margin);
                link.end = link.end.saturating_add(margin);
            }
        }
        if text
            .len()
            .checked_add(line.text.len().saturating_add(1))
            .is_none_or(|length| length > MAX_OUTPUT_BYTES)
        {
            return Err(RenderError::OutputExpansion);
        }
        text.push_str(&line.text);
        text.push('\n');
        spans.push(line.spans);
        links.push(line.links);
    }
    while text.ends_with("\n\n") {
        text.pop();
        spans.pop();
        links.pop();
    }
    Ok(MarkdownDocument { text, spans, links })
}

fn fill_heading_band(mut line: AnnotatedLine, width: usize) -> AnnotatedLine {
    if !line
        .spans
        .iter()
        .any(|span| span.style == SpanStyle::PreviewHeading1)
    {
        return line;
    }
    let used = text_layout::cell_width_from(&line.text, 0);
    let padding = width.saturating_sub(used);
    line.text.push_str(&" ".repeat(padding));
    line.scalar_len = line.scalar_len.saturating_add(padding);
    line.spans
        .retain(|span| span.style != SpanStyle::PreviewHeading1);
    line.spans.insert(
        0,
        StyledSpan {
            start: 0,
            end: line.scalar_len,
            style: SpanStyle::PreviewHeading1,
        },
    );
    line
}

fn distribute_spans(lines: &mut [AnnotatedLine], line_starts: &[usize], spans: &[GlobalSpan]) {
    for span in spans {
        let mut row = line_for_offset(line_starts, span.start);
        while let Some(line) = lines.get_mut(row) {
            let line_start = line_starts[row];
            let line_end = line_start.saturating_add(line.len());
            let start = span.start.max(line_start);
            let end = span.end.min(line_end);
            if start < end {
                line.spans.push(StyledSpan {
                    start: start - line_start,
                    end: end - line_start,
                    style: span.style,
                });
            }
            if span.end <= line_end.saturating_add(1) {
                break;
            }
            row += 1;
        }
    }
}

fn distribute_links(lines: &mut [AnnotatedLine], line_starts: &[usize], links: &[GlobalHyperlink]) {
    for link in links {
        let mut row = line_for_offset(line_starts, link.start);
        while let Some(line) = lines.get_mut(row) {
            let line_start = line_starts[row];
            let line_end = line_start.saturating_add(line.len());
            let start = link.start.max(line_start);
            let end = link.end.min(line_end);
            if start < end {
                line.links.push(HyperlinkSpan {
                    start: start - line_start,
                    end: end - line_start,
                    destination: Arc::clone(&link.destination),
                });
            }
            if link.end <= line_end.saturating_add(1) {
                break;
            }
            row += 1;
        }
    }
}

fn line_for_offset(line_starts: &[usize], offset: usize) -> usize {
    line_starts
        .partition_point(|line_start| *line_start <= offset)
        .saturating_sub(1)
}

fn wrap_line(line: AnnotatedLine, width: usize) -> Vec<AnnotatedLine> {
    if line.text.is_empty() {
        return vec![AnnotatedLine::empty()];
    }
    if text_layout::cell_width_from(&line.text, 0) <= width {
        return vec![line];
    }
    let mut line = line;
    line.chars = line.text.chars().collect();
    let quote_end = 0;
    let rest = &line.chars[quote_end..];

    let indent = rest.iter().take_while(|ch| **ch == ' ').count();
    let after_indent = &rest[indent..];
    if let Some(marker_len) = list_marker_len(after_indent) {
        let content_start = quote_end + indent + marker_len;
        return wrap_prefixed(
            &line,
            content_start,
            content_start,
            quote_end,
            indent + marker_len,
            width,
            false,
        );
    }

    let content_start = quote_end + indent;
    wrap_prefixed(
        &line,
        content_start,
        content_start,
        quote_end,
        indent,
        width,
        false,
    )
}

fn list_marker_len(chars: &[char]) -> Option<usize> {
    let marker = if chars.starts_with(&['•', ' ']) {
        2
    } else {
        let digits = chars.iter().take_while(|ch| ch.is_ascii_digit()).count();
        if digits == 0 || chars.get(digits) != Some(&'.') || chars.get(digits + 1) != Some(&' ') {
            return None;
        }
        digits + 2
    };
    let task = chars.get(marker..marker + 4).is_some_and(|task| {
        task[0] == '[' && matches!(task[1], '✓' | ' ') && task[2] == ']' && task[3] == ' '
    });
    Some(marker + if task { 4 } else { 0 })
}

#[allow(clippy::too_many_arguments)]
fn wrap_prefixed(
    source: &AnnotatedLine,
    content_start: usize,
    first_prefix_end: usize,
    quote_end: usize,
    continuation_spaces: usize,
    width: usize,
    preserve_spacing: bool,
) -> Vec<AnnotatedLine> {
    let first = prefixed_line(source, first_prefix_end, 0, width);
    let continuation = || prefixed_line(source, quote_end, continuation_spaces, width);
    if preserve_spacing {
        wrap_preserved(source, content_start, width, first, continuation)
    } else {
        wrap_words(source, content_start, width, first, continuation)
    }
}

fn prefixed_line(
    source: &AnnotatedLine,
    source_prefix_end: usize,
    extra_spaces: usize,
    width: usize,
) -> LineBuilder {
    let prefix = source.slice_text(0, source_prefix_end);
    let prefix_width = text_layout::cell_width_from(&prefix, 0).saturating_add(extra_spaces);
    let mut line = LineBuilder::new();
    if prefix_width < width {
        line.push_slice(source, 0, source_prefix_end);
        line.push_plain(&" ".repeat(extra_spaces));
    }
    line
}

fn wrap_words(
    source: &AnnotatedLine,
    content_start: usize,
    width: usize,
    mut current: LineBuilder,
    continuation: impl Fn() -> LineBuilder,
) -> Vec<AnnotatedLine> {
    let words = word_ranges(&source.chars, content_start);
    if words.is_empty() {
        return vec![current.finish()];
    }
    let mut output = Vec::new();
    let mut has_content = false;
    for (word_start, word_end) in words {
        let word = source.slice_text(word_start, word_end);
        let separator = usize::from(has_content);
        let needed = text_layout::cell_width_from(&word, 0).saturating_add(separator);
        if has_content && needed > width.saturating_sub(current.width()) {
            output.push(current.finish());
            current = continuation();
            has_content = false;
        }
        if has_content {
            current.push_plain(" ");
        }
        let mut start = word_start;
        while start < word_end {
            let available = width.saturating_sub(current.width());
            let end = fitting_end(source, start, word_end, available);
            if end == start {
                let consumed = next_grapheme_end(source, start, word_end);
                current.push_replacement(source, start, "…");
                start = consumed;
            } else {
                current.push_slice(source, start, end);
                start = end;
            }
            has_content = true;
            if start < word_end {
                output.push(current.finish());
                current = continuation();
                has_content = false;
            }
        }
    }
    output.push(current.finish());
    output
}

fn wrap_preserved(
    source: &AnnotatedLine,
    content_start: usize,
    width: usize,
    mut current: LineBuilder,
    continuation: impl Fn() -> LineBuilder,
) -> Vec<AnnotatedLine> {
    let mut output = Vec::new();
    let mut start = content_start;
    while start < source.len() {
        let available = width.saturating_sub(current.width());
        let end = fitting_end(source, start, source.len(), available);
        if end == start {
            let consumed = next_grapheme_end(source, start, source.len());
            current.push_replacement(source, start, "…");
            start = consumed;
        } else {
            current.push_slice(source, start, end);
            start = end;
        }
        if start < source.len() {
            output.push(current.finish());
            current = continuation();
        }
    }
    output.push(current.finish());
    output
}

fn word_ranges(chars: &[char], start: usize) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut word_start = None;
    for (index, ch) in chars.iter().enumerate().skip(start) {
        if ch.is_whitespace() {
            if let Some(start) = word_start.take() {
                words.push((start, index));
            }
        } else if word_start.is_none() {
            word_start = Some(index);
        }
    }
    if let Some(start) = word_start {
        words.push((start, chars.len()));
    }
    words
}

fn fitting_end(source: &AnnotatedLine, start: usize, end: usize, max_cells: usize) -> usize {
    if max_cells == 0 {
        return start;
    }
    let text = source.slice_text(start, end);
    let mut cells = 0;
    let mut scalars = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = text_layout::cell_width_from(grapheme, cells);
        if cells.saturating_add(grapheme_width) > max_cells {
            break;
        }
        cells = cells.saturating_add(grapheme_width);
        scalars += grapheme.chars().count();
    }
    start + scalars
}

fn next_grapheme_end(source: &AnnotatedLine, start: usize, end: usize) -> usize {
    let text = source.slice_text(start, end);
    start
        + text
            .graphemes(true)
            .next()
            .map(str::chars)
            .map(Iterator::count)
            .unwrap_or(0)
}

#[cfg(test)]
mod tests;
