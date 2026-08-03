//! Purpose: present Markdown source as a bounded, readable terminal document.
//! Owns: shared pulldown-cmark interpretation, width-aware layout, and semantic styling.
//! Must not: read files, emit ANSI, mutate source buffers, or run during ordinary typing.
//! Invariants: conversion is explicit; every produced line is bounded to the reading width.

use std::borrow::Cow;
#[cfg(test)]
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::{CompactLineStarts, PreviewBuffer};
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
    line_starts: CompactLineStarts,
    pub(crate) annotations: MarkdownAnnotations,
}

impl MarkdownDocument {
    pub(crate) fn into_buffer_and_annotations(self) -> (PreviewBuffer, MarkdownAnnotations) {
        (
            PreviewBuffer::from_parts(self.text, self.line_starts),
            self.annotations,
        )
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.text
            .capacity()
            .saturating_add(self.line_starts.retained_bytes())
            .saturating_add(self.annotations.retained_bytes())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompactSpan {
    start: u32,
    end: u32,
    style: SpanStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompactLink {
    start: u32,
    end: u32,
    destination: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompactAnnotationRow {
    row: u32,
    span_start: u32,
    span_end: u32,
    link_start: u32,
    link_end: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WideAnnotationRow {
    row: usize,
    span_start: usize,
    span_end: usize,
    link_start: usize,
    link_end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AnnotationStorage {
    Compact {
        rows: Vec<CompactAnnotationRow>,
        spans: Vec<CompactSpan>,
        links: Vec<CompactLink>,
    },
    Wide {
        rows: Vec<WideAnnotationRow>,
        spans: Vec<StyledSpan>,
        links: Vec<HyperlinkSpan>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkdownAnnotations {
    storage: AnnotationStorage,
}

impl Default for MarkdownAnnotations {
    fn default() -> Self {
        Self {
            storage: AnnotationStorage::Compact {
                rows: Vec::new(),
                spans: Vec::new(),
                links: Vec::new(),
            },
        }
    }
}

impl MarkdownAnnotations {
    pub(crate) fn spans(&self, row: usize) -> RowSpans<'_> {
        match &self.storage {
            AnnotationStorage::Compact { rows, spans, .. } => {
                let Some(annotation_row) = rows
                    .binary_search_by_key(&row, |candidate| candidate.row as usize)
                    .ok()
                    .and_then(|index| rows.get(index))
                else {
                    return RowSpans::Empty;
                };
                RowSpans::Compact(
                    spans[annotation_row.span_start as usize..annotation_row.span_end as usize]
                        .iter(),
                )
            }
            AnnotationStorage::Wide { rows, spans, .. } => {
                let Some(annotation_row) = rows
                    .binary_search_by_key(&row, |candidate| candidate.row)
                    .ok()
                    .and_then(|index| rows.get(index))
                else {
                    return RowSpans::Empty;
                };
                RowSpans::Wide(spans[annotation_row.span_start..annotation_row.span_end].iter())
            }
        }
    }

    pub(crate) fn links(&self, row: usize) -> RowLinks<'_> {
        match &self.storage {
            AnnotationStorage::Compact { rows, links, .. } => {
                let Some(annotation_row) = rows
                    .binary_search_by_key(&row, |candidate| candidate.row as usize)
                    .ok()
                    .and_then(|index| rows.get(index))
                else {
                    return RowLinks::Empty;
                };
                RowLinks::Compact(
                    links[annotation_row.link_start as usize..annotation_row.link_end as usize]
                        .iter(),
                )
            }
            AnnotationStorage::Wide { rows, links, .. } => {
                let Some(annotation_row) = rows
                    .binary_search_by_key(&row, |candidate| candidate.row)
                    .ok()
                    .and_then(|index| rows.get(index))
                else {
                    return RowLinks::Empty;
                };
                RowLinks::Wide(links[annotation_row.link_start..annotation_row.link_end].iter())
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn from_rows(
        span_rows: &[Vec<StyledSpan>],
        link_rows: &[Vec<HyperlinkSpan>],
    ) -> Self {
        let mut builder = AnnotationBuilder::default();
        let row_count = span_rows.len().max(link_rows.len());
        for row in 0..row_count {
            builder
                .push_row(
                    row,
                    span_rows.get(row).cloned().unwrap_or_default(),
                    link_rows.get(row).cloned().unwrap_or_default(),
                )
                .expect("test annotations fit configured cap");
        }
        builder.finish()
    }

    #[cfg(test)]
    pub(crate) fn annotated_row_count(&self) -> usize {
        match &self.storage {
            AnnotationStorage::Compact { rows, .. } => rows.len(),
            AnnotationStorage::Wide { rows, .. } => rows.len(),
        }
    }

    #[cfg(test)]
    pub(crate) fn annotation_count(&self) -> usize {
        match &self.storage {
            AnnotationStorage::Compact { spans, links, .. } => spans.len() + links.len(),
            AnnotationStorage::Wide { spans, links, .. } => spans.len() + links.len(),
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.container_retained_bytes()
            .saturating_add(self.link_destination_retained_bytes())
    }

    #[cfg(test)]
    fn container_retained_bytes(&self) -> usize {
        match &self.storage {
            AnnotationStorage::Compact { rows, spans, links } => rows
                .capacity()
                .saturating_mul(std::mem::size_of::<CompactAnnotationRow>())
                .saturating_add(
                    spans
                        .capacity()
                        .saturating_mul(std::mem::size_of::<CompactSpan>()),
                )
                .saturating_add(
                    links
                        .capacity()
                        .saturating_mul(std::mem::size_of::<CompactLink>()),
                ),
            AnnotationStorage::Wide { rows, spans, links } => rows
                .capacity()
                .saturating_mul(std::mem::size_of::<WideAnnotationRow>())
                .saturating_add(
                    spans
                        .capacity()
                        .saturating_mul(std::mem::size_of::<StyledSpan>()),
                )
                .saturating_add(
                    links
                        .capacity()
                        .saturating_mul(std::mem::size_of::<HyperlinkSpan>()),
                ),
        }
    }

    #[cfg(test)]
    fn link_destination_retained_bytes(&self) -> usize {
        match &self.storage {
            AnnotationStorage::Compact { links, .. } => {
                unique_arc_str_retained_bytes(links.iter().map(|link| &link.destination))
            }
            AnnotationStorage::Wide { links, .. } => {
                unique_arc_str_retained_bytes(links.iter().map(|link| &link.destination))
            }
        }
    }
}

#[cfg(test)]
fn unique_arc_str_retained_bytes<'a>(destinations: impl Iterator<Item = &'a Arc<str>>) -> usize {
    let mut seen = HashSet::new();
    destinations.fold(0usize, |bytes, destination| {
        let allocation = Arc::as_ptr(destination) as *const u8 as usize;
        if seen.insert(allocation) {
            bytes.saturating_add(arc_str_allocation_bytes(destination))
        } else {
            bytes
        }
    })
}

#[cfg(test)]
fn arc_str_allocation_bytes(destination: &Arc<str>) -> usize {
    let alignment = std::mem::align_of::<usize>();
    let allocation = std::mem::size_of::<usize>()
        .saturating_mul(2)
        .saturating_add(destination.len());
    allocation.saturating_add(alignment.saturating_sub(1)) / alignment * alignment
}

pub(crate) enum RowSpans<'a> {
    Empty,
    Compact(std::slice::Iter<'a, CompactSpan>),
    Wide(std::slice::Iter<'a, StyledSpan>),
}

impl Iterator for RowSpans<'_> {
    type Item = StyledSpan;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Empty => None,
            Self::Compact(spans) => spans.next().map(|span| StyledSpan {
                start: span.start as usize,
                end: span.end as usize,
                style: span.style,
            }),
            Self::Wide(spans) => spans.next().copied(),
        }
    }
}

pub(crate) enum RowLinks<'a> {
    Empty,
    Compact(std::slice::Iter<'a, CompactLink>),
    Wide(std::slice::Iter<'a, HyperlinkSpan>),
}

impl Iterator for RowLinks<'_> {
    type Item = HyperlinkSpan;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Empty => None,
            Self::Compact(links) => links.next().map(|link| HyperlinkSpan {
                start: link.start as usize,
                end: link.end as usize,
                destination: Arc::clone(&link.destination),
            }),
            Self::Wide(links) => links.next().cloned(),
        }
    }
}

#[derive(Default)]
struct AnnotationBuilder {
    annotations: MarkdownAnnotations,
}

impl AnnotationBuilder {
    fn push_row(
        &mut self,
        row: usize,
        spans: Vec<StyledSpan>,
        links: Vec<HyperlinkSpan>,
    ) -> Result<(), RenderError> {
        if spans.is_empty() && links.is_empty() {
            return Ok(());
        }
        let current = self.annotations.annotation_count_for_build();
        if current
            .checked_add(spans.len())
            .and_then(|count| count.checked_add(links.len()))
            .is_none_or(|count| count > MAX_ANNOTATIONS)
        {
            return Err(RenderError::OutputExpansion);
        }
        if self.can_push_compact(row, &spans, &links) {
            self.push_compact(row, spans, links);
        } else {
            self.promote();
            self.push_wide(row, spans, links);
        }
        Ok(())
    }

    fn can_push_compact(&self, row: usize, spans: &[StyledSpan], links: &[HyperlinkSpan]) -> bool {
        let AnnotationStorage::Compact {
            rows,
            spans: stored_spans,
            links: stored_links,
        } = &self.annotations.storage
        else {
            return false;
        };
        u32::try_from(row).is_ok()
            && u32::try_from(rows.len()).is_ok()
            && u32::try_from(stored_spans.len().saturating_add(spans.len())).is_ok()
            && u32::try_from(stored_links.len().saturating_add(links.len())).is_ok()
            && spans
                .iter()
                .all(|span| u32::try_from(span.start).is_ok() && u32::try_from(span.end).is_ok())
            && links
                .iter()
                .all(|link| u32::try_from(link.start).is_ok() && u32::try_from(link.end).is_ok())
    }

    fn push_compact(
        &mut self,
        row: usize,
        new_spans: Vec<StyledSpan>,
        new_links: Vec<HyperlinkSpan>,
    ) {
        let AnnotationStorage::Compact { rows, spans, links } = &mut self.annotations.storage
        else {
            unreachable!("compact eligibility checked before insertion");
        };
        let span_start = spans.len() as u32;
        spans.extend(new_spans.into_iter().map(|span| CompactSpan {
            start: span.start as u32,
            end: span.end as u32,
            style: span.style,
        }));
        let link_start = links.len() as u32;
        links.extend(new_links.into_iter().map(|link| CompactLink {
            start: link.start as u32,
            end: link.end as u32,
            destination: link.destination,
        }));
        rows.push(CompactAnnotationRow {
            row: row as u32,
            span_start,
            span_end: spans.len() as u32,
            link_start,
            link_end: links.len() as u32,
        });
    }

    fn promote(&mut self) {
        if matches!(self.annotations.storage, AnnotationStorage::Wide { .. }) {
            return;
        }
        let AnnotationStorage::Compact { rows, spans, links } = std::mem::replace(
            &mut self.annotations.storage,
            AnnotationStorage::Wide {
                rows: Vec::new(),
                spans: Vec::new(),
                links: Vec::new(),
            },
        ) else {
            unreachable!("wide storage returned before compact promotion");
        };
        self.annotations.storage = AnnotationStorage::Wide {
            rows: rows
                .into_iter()
                .map(|row| WideAnnotationRow {
                    row: row.row as usize,
                    span_start: row.span_start as usize,
                    span_end: row.span_end as usize,
                    link_start: row.link_start as usize,
                    link_end: row.link_end as usize,
                })
                .collect(),
            spans: spans
                .into_iter()
                .map(|span| StyledSpan {
                    start: span.start as usize,
                    end: span.end as usize,
                    style: span.style,
                })
                .collect(),
            links: links
                .into_iter()
                .map(|link| HyperlinkSpan {
                    start: link.start as usize,
                    end: link.end as usize,
                    destination: link.destination,
                })
                .collect(),
        };
    }

    fn push_wide(&mut self, row: usize, new_spans: Vec<StyledSpan>, new_links: Vec<HyperlinkSpan>) {
        let AnnotationStorage::Wide { rows, spans, links } = &mut self.annotations.storage else {
            unreachable!("wide storage created before insertion");
        };
        let span_start = spans.len();
        spans.extend(new_spans);
        let link_start = links.len();
        links.extend(new_links);
        rows.push(WideAnnotationRow {
            row,
            span_start,
            span_end: spans.len(),
            link_start,
            link_end: links.len(),
        });
    }

    fn finish(self) -> MarkdownAnnotations {
        self.annotations
    }
}

impl MarkdownAnnotations {
    fn annotation_count_for_build(&self) -> usize {
        match &self.storage {
            AnnotationStorage::Compact { spans, links, .. } => spans.len() + links.len(),
            AnnotationStorage::Wide { spans, links, .. } => spans.len() + links.len(),
        }
    }
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
                let indent = heading_indent(level);
                if indent > 0 {
                    self.push(&" ".repeat(indent));
                }
                self.active_styles.push(heading_style(level));
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
        self.spans.sort_by_key(|span| span.start);
        self.hyperlinks.sort_by_key(|link| link.start);
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

    fn push_slice(&mut self, source: &WrappedSourceLine<'_>, start: usize, end: usize) {
        let start = start.min(source.len());
        let end = end.min(source.len());
        if start >= end {
            return;
        }
        let target = self.len;
        self.text.push_str(source.slice_text(start, end));
        self.len += end - start;
        for span in source.spans(start, end) {
            self.spans.push(StyledSpan {
                start: target + span.start - start,
                end: target + span.end - start,
                style: span.style,
            });
        }
        for link in source.links(start, end) {
            self.links.push(HyperlinkSpan {
                start: target + link.start - start,
                end: target + link.end - start,
                destination: link.destination,
            });
        }
    }

    fn push_plain(&mut self, text: &str) {
        self.text.push_str(text);
        self.len += text.chars().count();
    }

    fn push_replacement(&mut self, source: &WrappedSourceLine<'_>, col: usize, text: &str) {
        let start = self.len;
        self.push_plain(text);
        let end = self.len;
        for span in source.spans(col, col.saturating_add(1)) {
            self.spans.push(StyledSpan {
                start,
                end,
                style: span.style,
            });
        }
        if let Some(link) = source.links(col, col.saturating_add(1)).next() {
            self.links.push(HyperlinkSpan {
                start,
                end,
                destination: link.destination,
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

    fn finish(mut self) -> BuiltLine {
        self.trim_end();
        BuiltLine {
            text: self.text,
            spans: self.spans,
            links: self.links,
        }
    }
}

struct BuiltLine {
    text: String,
    spans: Vec<StyledSpan>,
    links: Vec<HyperlinkSpan>,
}

struct SourceLine<'a> {
    text: &'a str,
    global_start: usize,
    scalar_len: usize,
    spans: &'a [GlobalSpan],
    links: &'a [GlobalHyperlink],
}

impl SourceLine<'_> {
    fn local_spans(&self, start: usize, end: usize) -> impl Iterator<Item = StyledSpan> + '_ {
        let global_start = self.global_start.saturating_add(start.min(self.scalar_len));
        let global_end = self.global_start.saturating_add(end.min(self.scalar_len));
        self.spans.iter().filter_map(move |span| {
            let overlap_start = span.start.max(global_start);
            let overlap_end = span.end.min(global_end);
            (overlap_start < overlap_end).then_some(StyledSpan {
                start: overlap_start - self.global_start,
                end: overlap_end - self.global_start,
                style: span.style,
            })
        })
    }

    fn local_links(&self, start: usize, end: usize) -> impl Iterator<Item = HyperlinkSpan> + '_ {
        let global_start = self.global_start.saturating_add(start.min(self.scalar_len));
        let global_end = self.global_start.saturating_add(end.min(self.scalar_len));
        self.links.iter().filter_map(move |link| {
            let overlap_start = link.start.max(global_start);
            let overlap_end = link.end.min(global_end);
            (overlap_start < overlap_end).then_some(HyperlinkSpan {
                start: overlap_start - self.global_start,
                end: overlap_end - self.global_start,
                destination: Arc::clone(&link.destination),
            })
        })
    }
}

struct WrappedSourceLine<'a> {
    source: SourceLine<'a>,
    scalar_bytes: CompactLineStarts,
}

impl<'a> WrappedSourceLine<'a> {
    fn new(source: SourceLine<'a>) -> Self {
        let mut scalar_bytes = CompactLineStarts::new();
        for (byte, ch) in source.text.char_indices() {
            scalar_bytes.push(byte.saturating_add(ch.len_utf8()));
        }
        Self {
            source,
            scalar_bytes,
        }
    }

    fn len(&self) -> usize {
        self.source.scalar_len
    }

    fn slice_text(&self, start: usize, end: usize) -> &'a str {
        let start = self
            .scalar_bytes
            .get(start.min(self.len()))
            .unwrap_or(self.source.text.len());
        let end = self
            .scalar_bytes
            .get(end.min(self.len()))
            .unwrap_or(self.source.text.len());
        &self.source.text[start..end]
    }

    fn spans(&self, start: usize, end: usize) -> impl Iterator<Item = StyledSpan> + '_ {
        self.source.local_spans(start, end)
    }

    fn links(&self, start: usize, end: usize) -> impl Iterator<Item = HyperlinkSpan> + '_ {
        self.source.local_links(start, end)
    }
}

struct DocumentBuilder {
    text: String,
    line_starts: CompactLineStarts,
    annotations: AnnotationBuilder,
    row: usize,
    margin: usize,
}

impl DocumentBuilder {
    fn new(capacity: usize, margin: usize) -> Self {
        Self {
            text: String::with_capacity(capacity.min(MAX_OUTPUT_BYTES)),
            line_starts: CompactLineStarts::new(),
            annotations: AnnotationBuilder::default(),
            row: 0,
            margin,
        }
    }

    fn push_source_line(&mut self, source: &SourceLine<'_>) -> Result<(), RenderError> {
        let margin = usize::from(!source.text.is_empty()).saturating_mul(self.margin);
        let mut spans = source.local_spans(0, source.scalar_len).collect::<Vec<_>>();
        let links = source.local_links(0, source.scalar_len).collect::<Vec<_>>();
        consolidate_heading_span(&mut spans);
        shift_annotations(&mut spans, margin);
        let mut links = links;
        shift_links(&mut links, margin);
        self.ensure_room(
            margin
                .checked_add(source.text.len())
                .and_then(|length| length.checked_add(1))
                .ok_or(RenderError::OutputExpansion)?,
        )?;
        push_spaces(&mut self.text, margin);
        self.text.push_str(source.text);
        self.finish_row(spans, links)
    }

    fn push_built_line(&mut self, mut line: BuiltLine) -> Result<(), RenderError> {
        let margin = usize::from(!line.text.is_empty()).saturating_mul(self.margin);
        consolidate_heading_span(&mut line.spans);
        shift_annotations(&mut line.spans, margin);
        shift_links(&mut line.links, margin);
        self.ensure_room(
            margin
                .checked_add(line.text.len())
                .and_then(|length| length.checked_add(1))
                .ok_or(RenderError::OutputExpansion)?,
        )?;
        push_spaces(&mut self.text, margin);
        self.text.push_str(&line.text);
        self.finish_row(line.spans, line.links)
    }

    fn ensure_room(&self, additional: usize) -> Result<(), RenderError> {
        if self
            .text
            .len()
            .checked_add(additional)
            .is_none_or(|length| length > MAX_OUTPUT_BYTES)
        {
            Err(RenderError::OutputExpansion)
        } else {
            Ok(())
        }
    }

    fn finish_row(
        &mut self,
        spans: Vec<StyledSpan>,
        links: Vec<HyperlinkSpan>,
    ) -> Result<(), RenderError> {
        self.annotations.push_row(self.row, spans, links)?;
        self.text.push('\n');
        self.row = self.row.saturating_add(1);
        self.line_starts.push(self.text.len());
        Ok(())
    }

    fn finish(mut self) -> MarkdownDocument {
        while self.text.ends_with("\n\n") {
            self.text.pop();
            self.line_starts.pop();
        }
        MarkdownDocument {
            text: self.text,
            line_starts: self.line_starts,
            annotations: self.annotations.finish(),
        }
    }
}

fn consolidate_heading_span(spans: &mut Vec<StyledSpan>) {
    let Some(style) = spans.iter().find_map(|span| match span.style {
        SpanStyle::PreviewHeading1
        | SpanStyle::PreviewHeading2
        | SpanStyle::PreviewHeading3
        | SpanStyle::PreviewHeading4
        | SpanStyle::PreviewHeading5
        | SpanStyle::PreviewHeading6 => Some(span.style),
        _ => None,
    }) else {
        return;
    };
    let mut start = usize::MAX;
    let mut end = 0usize;
    spans.retain(|span| {
        if span.style == style {
            start = start.min(span.start);
            end = end.max(span.end);
            false
        } else {
            true
        }
    });
    if start < end {
        spans.push(StyledSpan { start, end, style });
        spans.sort_by_key(|span| span.start);
    }
}

fn shift_annotations(spans: &mut [StyledSpan], amount: usize) {
    for span in spans {
        span.start = span.start.saturating_add(amount);
        span.end = span.end.saturating_add(amount);
    }
}

fn shift_links(links: &mut [HyperlinkSpan], amount: usize) {
    for link in links {
        link.start = link.start.saturating_add(amount);
        link.end = link.end.saturating_add(amount);
    }
}

fn push_spaces(text: &mut String, count: usize) {
    text.extend(std::iter::repeat_n(' ', count));
}

fn wrap_document(
    document: RawDocument,
    width: usize,
    margin: usize,
) -> Result<MarkdownDocument, RenderError> {
    if document.text.is_empty() {
        return Ok(MarkdownDocument {
            text: String::new(),
            line_starts: CompactLineStarts::new(),
            annotations: MarkdownAnnotations::default(),
        });
    }
    let mut output = DocumentBuilder::new(document.text.len(), margin);
    let mut global_start = 0usize;
    let mut span_start = 0usize;
    let mut link_start = 0usize;
    // The builder already terminates every emitted row. Omitting the synthetic
    // empty item after a trailing newline avoids crossing capacity by one byte
    // and doubling the canonical output allocation for newline-dense documents.
    for text in document.text.split_terminator('\n') {
        let scalar_len = text.chars().count();
        let global_end = global_start.saturating_add(scalar_len);
        while document
            .spans
            .get(span_start)
            .is_some_and(|span| span.end <= global_start)
        {
            span_start += 1;
        }
        while document
            .hyperlinks
            .get(link_start)
            .is_some_and(|link| link.end <= global_start)
        {
            link_start += 1;
        }
        let span_end = span_start
            + document.spans[span_start..].partition_point(|span| span.start < global_end);
        let link_end = link_start
            + document.hyperlinks[link_start..].partition_point(|link| link.start < global_end);
        let source = SourceLine {
            text,
            global_start,
            scalar_len,
            spans: &document.spans[span_start..span_end],
            links: &document.hyperlinks[link_start..link_end],
        };
        wrap_line(source, width, &mut output)?;
        global_start = global_end.saturating_add(1);
    }
    Ok(output.finish())
}

fn wrap_line(
    line: SourceLine<'_>,
    width: usize,
    output: &mut DocumentBuilder,
) -> Result<(), RenderError> {
    if line.text.is_empty() || text_layout::cell_width_from(line.text, 0) <= width {
        return output.push_source_line(&line);
    }
    let line = WrappedSourceLine::new(line);
    let quote_end = 0;
    let rest = line.slice_text(quote_end, line.len());
    let indent = rest.chars().take_while(|ch| *ch == ' ').count();
    let after_indent = line.slice_text(quote_end.saturating_add(indent), line.len());
    if let Some(marker_len) = list_marker_len(after_indent) {
        let content_start = quote_end + indent + marker_len;
        return wrap_prefixed(
            &line,
            content_start,
            content_start,
            quote_end,
            indent + marker_len,
            width,
            output,
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
        output,
    )
}

fn list_marker_len(text: &str) -> Option<usize> {
    let marker = if text.starts_with("• ") {
        2
    } else {
        let digits = text.chars().take_while(char::is_ascii_digit).count();
        let mut tail = text.chars().skip(digits);
        if digits == 0 || tail.next() != Some('.') || tail.next() != Some(' ') {
            return None;
        }
        digits + 2
    };
    let mut task = text.chars().skip(marker);
    let task = task.next() == Some('[')
        && matches!(task.next(), Some('✓' | ' '))
        && task.next() == Some(']')
        && task.next() == Some(' ');
    Some(marker + if task { 4 } else { 0 })
}

#[allow(clippy::too_many_arguments)]
fn wrap_prefixed(
    source: &WrappedSourceLine<'_>,
    content_start: usize,
    first_prefix_end: usize,
    quote_end: usize,
    continuation_spaces: usize,
    width: usize,
    output: &mut DocumentBuilder,
) -> Result<(), RenderError> {
    let first = prefixed_line(source, first_prefix_end, 0, width);
    let continuation = || prefixed_line(source, quote_end, continuation_spaces, width);
    wrap_words(source, content_start, width, first, continuation, output)
}

fn prefixed_line(
    source: &WrappedSourceLine<'_>,
    source_prefix_end: usize,
    extra_spaces: usize,
    width: usize,
) -> LineBuilder {
    let prefix = source.slice_text(0, source_prefix_end);
    let prefix_width = text_layout::cell_width_from(prefix, 0).saturating_add(extra_spaces);
    let mut line = LineBuilder::new();
    if prefix_width < width {
        line.push_slice(source, 0, source_prefix_end);
        line.push_plain(&" ".repeat(extra_spaces));
    }
    line
}

fn wrap_words(
    source: &WrappedSourceLine<'_>,
    content_start: usize,
    width: usize,
    current: LineBuilder,
    continuation: impl Fn() -> LineBuilder,
    output: &mut DocumentBuilder,
) -> Result<(), RenderError> {
    let mut current = Some(current);
    let mut has_content = false;
    let mut word_start = None;
    for (index, ch) in source.source.text.chars().enumerate().skip(content_start) {
        if ch.is_whitespace() {
            if let Some(word_start) = word_start.take() {
                push_word(
                    source,
                    word_start,
                    index,
                    width,
                    &mut current,
                    &mut has_content,
                    &continuation,
                    output,
                )?;
            }
        } else if word_start.is_none() {
            word_start = Some(index);
        }
    }
    if let Some(word_start) = word_start {
        push_word(
            source,
            word_start,
            source.len(),
            width,
            &mut current,
            &mut has_content,
            &continuation,
            output,
        )?;
    }
    output.push_built_line(current.take().unwrap_or_else(LineBuilder::new).finish())
}

#[allow(clippy::too_many_arguments)]
fn push_word(
    source: &WrappedSourceLine<'_>,
    word_start: usize,
    word_end: usize,
    width: usize,
    current: &mut Option<LineBuilder>,
    has_content: &mut bool,
    continuation: &impl Fn() -> LineBuilder,
    output: &mut DocumentBuilder,
) -> Result<(), RenderError> {
    let word = source.slice_text(word_start, word_end);
    let separator = usize::from(*has_content);
    let needed = text_layout::cell_width_from(word, 0).saturating_add(separator);
    let remaining = width.saturating_sub(current.as_ref().map_or(0, LineBuilder::width));
    if *has_content && needed > remaining {
        output.push_built_line(current.take().unwrap_or_else(LineBuilder::new).finish())?;
        *current = Some(continuation());
        *has_content = false;
    }
    if *has_content {
        current
            .as_mut()
            .unwrap_or_else(|| unreachable!("line builder is always present"))
            .push_plain(" ");
    }
    let mut start = word_start;
    while start < word_end {
        let line = current
            .as_mut()
            .unwrap_or_else(|| unreachable!("line builder is always present"));
        let available = width.saturating_sub(line.width());
        let end = fitting_end(source, start, word_end, available);
        if end == start {
            let consumed = next_grapheme_end(source, start, word_end);
            line.push_replacement(source, start, "…");
            start = consumed;
        } else {
            line.push_slice(source, start, end);
            start = end;
        }
        *has_content = true;
        if start < word_end {
            output.push_built_line(current.take().unwrap_or_else(LineBuilder::new).finish())?;
            *current = Some(continuation());
            *has_content = false;
        }
    }
    Ok(())
}

fn fitting_end(
    source: &WrappedSourceLine<'_>,
    start: usize,
    end: usize,
    max_cells: usize,
) -> usize {
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

fn next_grapheme_end(source: &WrappedSourceLine<'_>, start: usize, end: usize) -> usize {
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
