//! Purpose: convert one active Markdown buffer/page into readable terminal preview text.
//! Owns: pulldown-cmark block/inline interpretation and explicit preview layout.
//! Must not: read files, emit ANSI, mutate source buffers, or run during ordinary typing.
//! Invariants: conversion is explicit; tables retain structure before cell-width rendering.
//! Phase: F6 Markdown preview, expanded for issue #54.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

mod table;

pub(crate) fn render(source: &str) -> String {
    let mut options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    if source.contains("[^") {
        options.insert(Options::ENABLE_FOOTNOTES);
    }
    let mut renderer = PreviewRenderer::default();
    for event in Parser::new_ext(source, options) {
        renderer.event(event);
    }
    renderer.finish()
}

#[derive(Default)]
struct PreviewRenderer {
    output: String,
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    code_block: bool,
    table: Option<table::TableBuilder>,
    links: Vec<LinkTarget>,
}

struct LinkTarget {
    destination: String,
}

impl PreviewRenderer {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => self.push(&format!("‹{text}›")),
            Event::SoftBreak | Event::HardBreak => self.line_break(),
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
            TagEnd::Paragraph | TagEnd::Heading(_) => self.blank_line(),
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
            _ => "• ".to_string(),
        };
        self.push(&marker);
    }

    fn start_code_block(&mut self, kind: CodeBlockKind<'_>) {
        self.block_start();
        let language = match kind {
            CodeBlockKind::Fenced(language) if !language.is_empty() => format!(": {language}"),
            _ => String::new(),
        };
        self.push(&format!("┌─ code{language}"));
        self.newline();
        self.code_block = true;
    }

    fn end_code_block(&mut self) {
        self.newline();
        self.code_block = false;
        self.push("└─");
        self.blank_line();
    }

    fn start_link(&mut self, destination: String, image: bool) {
        if image {
            self.push("image: ");
        }
        self.links.push(LinkTarget { destination });
    }

    fn end_link(&mut self) {
        let Some(link) = self.links.pop() else {
            return;
        };
        self.push(&format!(" ⟨{}⟩", link.destination));
    }

    fn end_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        for line in table.finish() {
            self.push(&line);
            self.newline();
        }
        self.blank_line();
    }

    fn rule(&mut self) {
        self.block_start();
        self.push("────────────────────────");
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
        if let Some(table) = self.table.as_mut() {
            table.push(text);
            return;
        }
        if self.at_line_start() {
            self.push_prefix();
        }
        self.output.push_str(text);
    }

    fn push_prefix(&mut self) {
        if self.quote_depth > 0 {
            self.output.push_str(&"│ ".repeat(self.quote_depth));
        }
        if self.code_block {
            self.output.push_str("┊ ");
        }
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

    fn finish(mut self) -> String {
        while self.output.ends_with("\n\n") {
            self.output.pop();
        }
        self.output
    }
}

fn heading_marker(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "▌ ",
        HeadingLevel::H2 => "▌▌ ",
        HeadingLevel::H3 => "▌▌▌ ",
        HeadingLevel::H4 => "▌▌▌▌ ",
        HeadingLevel::H5 => "▌▌▌▌▌ ",
        HeadingLevel::H6 => "▌▌▌▌▌▌ ",
    }
}

#[cfg(test)]
mod tests;
