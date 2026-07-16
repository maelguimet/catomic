//! Purpose: convert one active Markdown buffer/page into readable terminal preview text.
//! Owns: pulldown-cmark event interpretation and block/list/quote text layout.
//! Must not: read files, emit ANSI, mutate source buffers, wrap to terminal width, or network.
//! Invariants: conversion is explicit on preview entry; output is valid UTF-8 with stable lines.
//! Phase: 4-c toggleable Markdown preview.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub(crate) fn render(source: &str) -> String {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
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
    table_cell: usize,
}

impl PreviewRenderer {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => self.push(&format!("‹{text}›")),
            Event::SoftBreak | Event::HardBreak => self.newline(),
            Event::Rule => {
                self.block_start();
                self.push("────────────────");
                self.blank_line();
            }
            Event::TaskListMarker(done) => self.push(if done { "[x] " } else { "[ ] " }),
            Event::Html(text) | Event::InlineHtml(text) => self.push(&text),
            Event::FootnoteReference(label) => self.push(&format!("[{label}]")),
            Event::InlineMath(text) => self.push(&format!("${text}$")),
            Event::DisplayMath(text) => {
                self.block_start();
                self.push(&text);
                self.blank_line();
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { .. } => {
                self.block_start();
                self.push("▌ ");
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
                self.block_start();
            }
            Tag::CodeBlock(_) => {
                self.block_start();
                self.code_block = true;
            }
            Tag::List(first) => self.lists.push(first),
            Tag::Item => {
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
            Tag::TableCell => {
                if self.table_cell > 0 {
                    self.push(" │ ");
                }
                self.table_cell += 1;
            }
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
            TagEnd::CodeBlock => {
                self.code_block = false;
                self.blank_line();
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.blank_line();
            }
            TagEnd::Item => self.newline(),
            TagEnd::TableRow => {
                self.table_cell = 0;
                self.newline();
            }
            TagEnd::Table => self.blank_line(),
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                self.newline();
            }
            if !part.is_empty() {
                self.push(part);
            }
        }
    }

    fn push(&mut self, text: &str) {
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
            self.output.push_str("  ");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_blocks_without_source_markers() {
        let source = "# Title\n\n> quote\n\n- one\n- two with `code`\n\n```rs\nlet x = 1;\n```";
        let preview = render(source);

        assert!(preview.contains("▌ Title"));
        assert!(preview.contains("│ quote"));
        assert!(preview.contains("• one"));
        assert!(preview.contains("• two with ‹code›"));
        assert!(preview.contains("  let x = 1;"));
        assert!(!preview.contains("# Title"));
        assert!(!preview.contains("```"));
    }

    #[test]
    fn renders_ordered_tasks_tables_and_unicode() {
        let source = "3. [x] café\n4. [ ] 猫\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let preview = render(source);

        assert!(preview.contains("3. [x] café"));
        assert!(preview.contains("4. [ ] 猫"));
        assert!(preview.contains("A │ B"));
        assert!(preview.contains("1 │ 2"));
    }
}
