//! Compact display-cell summaries over grapheme-aligned blocks.
//! Plain ASCII runs use arithmetic; other blocks target 4 KiB and never split
//! a grapheme. Queries visit a logarithmic summary path and at most one block.
//! Edits repair complete boundary blocks, extending through an affected grapheme
//! or regional-indicator run until the unchanged suffix segmentation agrees.

use std::borrow::Cow;
use std::io;
use std::ops::Range;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use crate::editor::text_layout::{
    cell_width_from, complete_grapheme_run_width, grapheme_width, TAB_WIDTH,
};

const BLOCK_BYTES: usize = 4096;
type Tree = Option<Box<Node>>;

#[derive(Clone, Copy, Debug, Default)]
struct Measure {
    cells: [usize; TAB_WIDTH],
    resets: bool,
}

impl Measure {
    fn apply(self, cell: usize) -> usize {
        let advance = self.cells[cell % TAB_WIDTH];
        if self.resets {
            advance
        } else {
            cell.saturating_add(advance)
        }
    }

    fn then(self, other: Self) -> Self {
        let resets = self.resets || other.resets;
        Self {
            cells: std::array::from_fn(|phase| {
                let end = other.apply(self.apply(phase));
                if resets {
                    end
                } else {
                    end.saturating_sub(phase)
                }
            }),
            resets,
        }
    }

    fn ascii(bytes: usize) -> Self {
        Self {
            cells: [bytes; TAB_WIDTH],
            resets: false,
        }
    }

    fn advance(&mut self, cells: usize) {
        for value in &mut self.cells {
            *value = value.saturating_add(cells);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Block {
    bytes: usize,
    last_grapheme: usize,
    ascii: bool,
    measure: Measure,
}

impl Block {
    fn ascii(bytes: usize) -> Self {
        Self {
            bytes,
            last_grapheme: 1,
            ascii: true,
            measure: Measure::ascii(bytes),
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    block: Block,
    priority: u64,
    left: Tree,
    right: Tree,
    bytes: usize,
    measure: Measure,
    nodes: usize,
}

impl Node {
    fn update(&mut self) {
        self.bytes = bytes(&self.left) + self.block.bytes + bytes(&self.right);
        self.measure = measure(&self.left)
            .then(self.block.measure)
            .then(measure(&self.right));
        self.nodes = nodes(&self.left) + 1 + nodes(&self.right);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CellIndex {
    root: Tree,
    priority: u64,
    error: Option<(io::ErrorKind, String)>,
    #[cfg(test)]
    work: std::cell::Cell<(usize, usize)>,
}

impl Default for CellIndex {
    fn default() -> Self {
        Self {
            root: None,
            priority: 0x9e37_79b9_7f4a_7c15,
            error: None,
            #[cfg(test)]
            work: Default::default(),
        }
    }
}

pub(crate) struct CellSplice {
    range: Range<usize>,
    blocks: Vec<Block>,
}

impl CellIndex {
    pub(crate) fn from_text(text: &str) -> Self {
        let mut builder = CellIndexBuilder::new();
        builder.push(text);
        builder.finish()
    }

    fn ensure_valid(&self) -> io::Result<()> {
        match &self.error {
            Some((kind, message)) => Err(io::Error::new(*kind, message.clone())),
            None => Ok(()),
        }
    }

    pub(crate) fn cell_at<'a>(
        &self,
        at: usize,
        mut read: impl FnMut(Range<usize>) -> io::Result<Cow<'a, str>>,
    ) -> io::Result<usize> {
        self.ensure_valid()?;
        let mut node = self.root.as_deref();
        let mut offset = 0;
        let mut cell = 0;
        while let Some(current) = node {
            self.record_work(1, 0);
            let start = offset + bytes(&current.left);
            if at < start {
                node = current.left.as_deref();
                continue;
            }
            cell = measure(&current.left).apply(cell);
            let end = start + current.block.bytes;
            if at < end {
                if current.block.ascii {
                    return Ok(cell + at.saturating_sub(start));
                }
                let text = read(start..end)?;
                self.record_work(0, text.len());
                let mut position = 0;
                for line in text.split_inclusive('\n') {
                    let content = line.strip_suffix('\n').unwrap_or(line);
                    for grapheme in
                        unicode_segmentation::UnicodeSegmentation::graphemes(content, true)
                    {
                        position += grapheme.len();
                        if position > at - start {
                            return Ok(cell);
                        }
                        cell += cell_width_from(grapheme, cell);
                    }
                    if line.ends_with('\n') {
                        position += 1;
                        if position > at - start {
                            return Ok(cell);
                        }
                        cell = 0;
                    }
                }
                return Ok(cell);
            }
            cell = current.block.measure.apply(cell);
            offset = end;
            node = current.right.as_deref();
        }
        Ok(cell)
    }

    pub(crate) fn prepare_splice<'a>(
        &self,
        range: Range<usize>,
        emit_inserted: impl FnOnce(&mut CellIndexBuilder) -> io::Result<()>,
        mut read: impl FnMut(Range<usize>) -> io::Result<Cow<'a, str>>,
    ) -> io::Result<CellSplice> {
        self.ensure_valid()?;
        let total = bytes(&self.root);
        let left = range.start.checked_sub(1).and_then(|at| self.locate(at));
        let right = self.locate(range.end);
        let start = left.map_or(
            0,
            |(offset, block)| {
                if block.ascii {
                    range.start - 1
                } else {
                    offset
                }
            },
        );
        let mut end = right.map_or(total, |(offset, block)| {
            if block.ascii {
                range.end + 1
            } else {
                offset + block.bytes
            }
        });
        let mut expected_last = right.map_or(0, |(_, block)| block.last_grapheme);
        let mut builder = CellIndexBuilder::new();
        if start < range.start {
            let text = read(start..range.start)?;
            self.record_work(0, text.len());
            builder.push(&text);
        }
        emit_inserted(&mut builder)?;
        let mut suffix_start = range.end;
        loop {
            if suffix_start < end {
                let text = read(suffix_start..end)?;
                self.record_work(0, text.len());
                builder.push(&text);
            }
            // Equal unchanged final graphemes restore the same segmentation
            // context, including regional-indicator parity. Otherwise extend
            // through the affected run rather than using a fixed scalar overlap.
            if end == total
                || (builder.last_grapheme_len() == expected_last
                    && expected_last <= end - range.end)
            {
                break;
            }
            suffix_start = end;
            let Some((offset, block)) = self.locate(end) else {
                break;
            };
            end = if block.ascii {
                offset + 1
            } else {
                offset + block.bytes
            };
            expected_last = block.last_grapheme;
        }
        Ok(CellSplice {
            range: start..end,
            blocks: builder.finish_blocks(),
        })
    }

    pub(crate) fn apply_splice(&mut self, splice: io::Result<CellSplice>) {
        let splice = match splice {
            Ok(splice) => splice,
            Err(error) => {
                // Existing infallible scalar edits can outlive a failed backing
                // read. Keep coordinate queries fallible instead of guessing a
                // column or silently rebuilding the whole line later.
                self.error = Some((error.kind(), error.to_string()));
                return;
            }
        };
        let (left, rest) = split(self.root.take(), splice.range.start, &mut self.priority);
        let (_, right) = split(rest, splice.range.len(), &mut self.priority);
        let mut middle = None;
        for block in splice.blocks {
            let node = new_node(block, &mut self.priority);
            middle = concat(middle, Some(node), &mut self.priority);
        }
        self.root = concat(
            concat(left, middle, &mut self.priority),
            right,
            &mut self.priority,
        );
    }

    fn locate(&self, at: usize) -> Option<(usize, Block)> {
        let mut node = self.root.as_deref();
        let mut offset = 0;
        while let Some(current) = node {
            self.record_work(1, 0);
            let start = offset + bytes(&current.left);
            if at < start {
                node = current.left.as_deref();
            } else if at < start + current.block.bytes {
                return Some((start, current.block));
            } else {
                offset = start + current.block.bytes;
                node = current.right.as_deref();
            }
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn take_work(&self) -> (usize, usize) {
        self.work.replace((0, 0))
    }
    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        nodes(&self.root) * std::mem::size_of::<Node>()
    }
    fn record_work(&self, visits: usize, read_bytes: usize) {
        #[cfg(test)]
        {
            let (old_visits, old_bytes) = self.work.get();
            self.work.set((old_visits + visits, old_bytes + read_bytes));
        }
        #[cfg(not(test))]
        let _ = (visits, read_bytes);
    }
}

/// Streaming builder retains only the unfinished grapheme, not the page text.
/// The Unicode cursor carries segmentation state across input chunk boundaries.
pub(crate) struct CellIndexBuilder {
    blocks: Vec<Block>,
    block: Block,
    pending: String,
    cursor: GraphemeCursor,
    pending_cr: bool,
}

impl CellIndexBuilder {
    pub(crate) fn new() -> Self {
        Self {
            blocks: Vec::new(),
            block: Block::default(),
            pending: String::new(),
            cursor: GraphemeCursor::new(0, usize::MAX, true),
            pending_cr: false,
        }
    }

    pub(crate) fn push_file_text(&mut self, text: &str) {
        let mut rest = text;
        if self.pending_cr {
            if !rest.starts_with('\n') {
                self.push("\r");
            }
            self.pending_cr = false;
        }
        while let Some(at) = rest.find('\r') {
            self.push(&rest[..at]);
            rest = &rest[at + 1..];
            if rest.is_empty() {
                self.pending_cr = true;
                return;
            }
            if !rest.starts_with('\n') {
                self.push("\r");
            }
        }
        self.push(rest);
    }

    pub(crate) fn push(&mut self, text: &str) {
        if text.len() > 2
            && text.is_ascii()
            && memchr::memchr3(b'\t', b'\r', b'\n', text.as_bytes()).is_none()
        {
            self.push_plain_ascii(text);
            return;
        }
        for part in text.split_inclusive('\n') {
            if let Some(content) = part.strip_suffix('\n') {
                self.push_chunks(content);
                self.flush_pending();
                self.push_grapheme("\n");
                self.cursor = GraphemeCursor::new(0, usize::MAX, true);
            } else {
                self.push_chunks(part);
            }
        }
    }

    fn push_chunks(&mut self, mut text: &str) {
        while !text.is_empty() {
            let mut end = text.len().min(64 * 1024);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.push_segment(&text[..end]);
            text = &text[end..];
        }
    }

    fn push_segment(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // Two plain ASCII scalars reset every Unicode joining context. Keep the
        // final scalar pending in case the next chunk begins with an extender.
        if text.len() > 2
            && text.is_ascii()
            && memchr::memchr2(b'\t', b'\r', text.as_bytes()).is_none()
        {
            self.push_plain_ascii(text);
            return;
        }
        let batch = self.pending.len() < BLOCK_BYTES && text.len() >= BLOCK_BYTES;
        self.pending.push_str(text);
        let mut combined = std::mem::take(&mut self.pending);
        if batch {
            let mut end_cursor = GraphemeCursor::new(combined.len(), combined.len(), true);
            let complete_end = end_cursor.prev_boundary(&combined, 0).unwrap().unwrap_or(0);
            if complete_end > 0 {
                self.push_complete_text(&combined[..complete_end]);
                combined.drain(..complete_end);
                self.cursor = GraphemeCursor::new(0, usize::MAX, true);
                // Initialize the retained final grapheme's streaming context.
                let result = self.cursor.next_boundary(&combined, 0);
                debug_assert_eq!(result, Err(GraphemeIncomplete::NextChunk));
                self.pending = combined;
                return;
            }
        }
        let mut consumed = 0;
        loop {
            // The retained chunk begins at the current complete grapheme start.
            // Rebase the cursor at each boundary: RI pairs restore even parity,
            // and all other grapheme rules depend only on this cluster's context.
            match self.cursor.next_boundary(&combined[consumed..], 0) {
                Ok(Some(boundary)) => {
                    self.push_grapheme(&combined[consumed..consumed + boundary]);
                    consumed += boundary;
                    self.cursor = GraphemeCursor::new(0, usize::MAX, true);
                }
                Err(GraphemeIncomplete::NextChunk) | Ok(None) => break,
                other => unreachable!("complete current-grapheme context: {other:?}"),
            }
        }
        combined.drain(..consumed);
        self.pending = combined;
    }

    fn push_plain_ascii(&mut self, text: &str) {
        self.push_segment(&text[..2]);
        self.flush_pending();
        self.cursor = GraphemeCursor::new(0, usize::MAX, true);
        self.push_ascii(text.len() - 3);
        self.push_segment(&text[text.len() - 1..]);
    }

    fn push_complete_text(&mut self, mut text: &str) {
        while !text.is_empty() {
            if self.block.bytes >= BLOCK_BYTES {
                self.flush_block();
            }
            let mut target = (BLOCK_BYTES - self.block.bytes).min(text.len());
            while !text.is_char_boundary(target) {
                target += 1;
            }
            let end = if target == text.len() {
                target
            } else {
                GraphemeCursor::new(target, text.len(), true)
                    .next_boundary(text, 0)
                    .unwrap()
                    .unwrap_or(text.len())
            };
            let run = &text[..end];
            let mut last = GraphemeCursor::new(run.len(), run.len(), true);
            self.block.last_grapheme = run.len() - last.prev_boundary(run, 0).unwrap().unwrap_or(0);
            self.block.bytes += run.len();
            self.block.ascii = false;
            let mut parts = run.split('\t');
            self.block
                .measure
                .advance(complete_grapheme_run_width(parts.next().unwrap()));
            for part in parts {
                self.block.measure = self.block.measure.then(Measure {
                    cells: std::array::from_fn(|phase| TAB_WIDTH - phase),
                    resets: false,
                });
                self.block
                    .measure
                    .advance(complete_grapheme_run_width(part));
            }
            text = &text[end..];
        }
    }

    fn flush_pending(&mut self) {
        if !self.pending.is_empty() {
            let text = std::mem::take(&mut self.pending);
            self.push_grapheme(&text);
            self.pending = text;
            self.pending.clear();
        }
    }

    fn push_ascii(&mut self, mut bytes: usize) {
        if bytes == 0 {
            return;
        }
        if self.block.bytes != 0 && !self.block.ascii {
            let take = bytes.min(BLOCK_BYTES.saturating_sub(self.block.bytes));
            self.block.bytes += take;
            self.block.measure.advance(take);
            if take > 0 {
                self.block.last_grapheme = 1;
            }
            bytes -= take;
            if bytes == 0 {
                return;
            }
            self.flush_block();
        }
        self.block = Block::ascii(self.block.bytes + bytes);
    }

    fn push_grapheme(&mut self, text: &str) {
        if text.len() == 1 && text.is_ascii() && !matches!(text, "\n" | "\t" | "\r") {
            self.push_ascii(1);
            return;
        }
        if self.block.bytes >= BLOCK_BYTES {
            self.flush_block();
        }
        self.block.ascii = false;
        if text == "\n" {
            self.block.measure = Measure {
                cells: [0; TAB_WIDTH],
                resets: true,
            };
        } else if text == "\t" {
            self.block.measure = self.block.measure.then(Measure {
                cells: std::array::from_fn(|phase| grapheme_width(text, phase)),
                resets: false,
            });
        } else {
            self.block.measure.advance(grapheme_width(text, 0));
        }
        self.block.bytes += text.len();
        self.block.last_grapheme = text.len();
    }

    fn flush_block(&mut self) {
        if self.block.bytes > 0 {
            self.blocks.push(std::mem::take(&mut self.block));
        }
    }

    fn last_grapheme_len(&self) -> usize {
        if self.pending.is_empty() {
            self.block.last_grapheme
        } else {
            self.pending.len()
        }
    }

    fn finish_blocks(mut self) -> Vec<Block> {
        if self.pending_cr {
            self.push("\r");
        }
        self.flush_pending();
        self.flush_block();
        self.blocks
    }

    pub(crate) fn finish(self) -> CellIndex {
        let mut index = CellIndex::default();
        for block in self.finish_blocks() {
            let node = new_node(block, &mut index.priority);
            index.root = concat(index.root.take(), Some(node), &mut index.priority);
        }
        index
    }
}

fn bytes(tree: &Tree) -> usize {
    tree.as_ref().map_or(0, |node| node.bytes)
}
fn nodes(tree: &Tree) -> usize {
    tree.as_ref().map_or(0, |node| node.nodes)
}
fn measure(tree: &Tree) -> Measure {
    tree.as_ref()
        .map_or(Measure::default(), |node| node.measure)
}
fn new_node(block: Block, priority: &mut u64) -> Box<Node> {
    *priority ^= *priority << 13;
    *priority ^= *priority >> 7;
    *priority ^= *priority << 17;
    Box::new(Node {
        block,
        priority: *priority,
        left: None,
        right: None,
        bytes: block.bytes,
        measure: block.measure,
        nodes: 1,
    })
}
fn merge(left: Tree, right: Tree) -> Tree {
    match (left, right) {
        (None, tree) | (tree, None) => tree,
        (Some(mut left), Some(mut right)) => {
            if left.priority >= right.priority {
                left.right = merge(left.right.take(), Some(right));
                left.update();
                Some(left)
            } else {
                right.left = merge(Some(left), right.left.take());
                right.update();
                Some(right)
            }
        }
    }
}
fn split(tree: Tree, at: usize, priority: &mut u64) -> (Tree, Tree) {
    let Some(mut node) = tree else {
        return (None, None);
    };
    let left_bytes = bytes(&node.left);
    if at < left_bytes {
        let (left, middle) = split(node.left.take(), at, priority);
        node.left = middle;
        node.update();
        (left, Some(node))
    } else if at > left_bytes + node.block.bytes {
        let (middle, right) = split(
            node.right.take(),
            at - left_bytes - node.block.bytes,
            priority,
        );
        node.right = middle;
        node.update();
        (Some(node), right)
    } else if at == left_bytes {
        let left = node.left.take();
        node.update();
        (left, Some(node))
    } else if at == left_bytes + node.block.bytes {
        let right = node.right.take();
        node.update();
        (Some(node), right)
    } else {
        assert!(
            node.block.ascii,
            "repairs split non-ASCII only at block boundaries"
        );
        let local = at - left_bytes;
        let left = Some(new_node(Block::ascii(local), priority));
        let right = Some(new_node(Block::ascii(node.block.bytes - local), priority));
        (
            merge(node.left.take(), left),
            merge(right, node.right.take()),
        )
    }
}
fn edge(tree: &Tree, last: bool) -> Option<Block> {
    let mut node = tree.as_deref()?;
    while let Some(next) = if last {
        node.right.as_deref()
    } else {
        node.left.as_deref()
    } {
        node = next;
    }
    Some(node.block)
}
fn concat(left: Tree, right: Tree, priority: &mut u64) -> Tree {
    match (edge(&left, true), edge(&right, false)) {
        (Some(a), Some(b)) if a.ascii && b.ascii => {
            let at = bytes(&left) - a.bytes;
            let (left, _) = split(left, at, priority);
            let (_, right) = split(right, b.bytes, priority);
            merge(
                merge(
                    left,
                    Some(new_node(Block::ascii(a.bytes + b.bytes), priority)),
                ),
                right,
            )
        }
        _ => merge(left, right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::text_layout::scalar_to_cell;

    fn check(index: &CellIndex, text: &str) {
        assert_eq!(bytes(&index.root), text.len());
        for at in text.char_indices().map(|(at, _)| at).chain([text.len()]) {
            let start = text[..at].rfind('\n').map_or(0, |at| at + 1);
            let end = text[at..].find('\n').map_or(text.len(), |end| at + end);
            let expected = scalar_to_cell(&text[start..end], text[start..at].chars().count());
            assert_eq!(
                index
                    .cell_at(at, |range| Ok(Cow::Borrowed(&text[range])))
                    .unwrap(),
                expected,
                "text={text:?} at={at}"
            );
        }
    }

    #[test]
    fn cold_and_edited_distant_queries_have_bounded_work() {
        for pattern in ["x", "猫e\u{301}👩\u{200d}💻\tabc"] {
            for size in [64 * 1024, 1024 * 1024, 4 * 1024 * 1024] {
                let mut text = pattern.repeat(size / pattern.len());
                let mut index = CellIndex::from_text(&text);
                assert!(index.retained_bytes() < text.len() / 8 + 1024);
                for edit in [false, true] {
                    if edit {
                        index.take_work();
                        let splice = index.prepare_splice(
                            0..0,
                            |builder| {
                                builder.push("λ");
                                Ok(())
                            },
                            |range| Ok(Cow::Borrowed(&text[range])),
                        );
                        index.apply_splice(splice);
                        text.insert(0, 'λ');
                        let (visits, read) = index.take_work();
                        assert!(visits < 128, "edit visits={visits}");
                        assert!(read <= 2 * BLOCK_BYTES + 64, "edit read={read}");
                    }
                    let expected = scalar_to_cell(&text, text.chars().count());
                    index.take_work();
                    let actual = index
                        .cell_at(text.len(), |range| Ok(Cow::Borrowed(&text[range])))
                        .unwrap();
                    let (visits, read) = index.take_work();
                    assert_eq!(actual, expected);
                    assert!(visits < 64, "query visits={visits}");
                    assert_eq!(read, 0, "cold End uses only initialized summaries");
                }
            }
        }
    }

    #[test]
    fn indexed_columns_match_rendering_at_every_scalar_and_chunk_boundary() {
        for text in [
            "",
            "abcdef",
            "abc\nxyz",
            "猫\te\u{301}👩\u{200d}💻",
            "🇫🇷🇺🇸🇯",
            "a\r\nb",
            "\u{600}ab",
            "क्‍कxy",
        ] {
            check(&CellIndex::from_text(text), text);
            let mut builder = CellIndexBuilder::new();
            for ch in text.chars() {
                builder.push(ch.encode_utf8(&mut [0; 4]));
            }
            check(&builder.finish(), text);
        }
    }

    #[test]
    fn batched_blocks_match_renderer_with_script_ligatures_and_controls() {
        // unicode-width can form these script ligatures across graphemes.
        // They must take the renderer fallback, including after dependency
        // upgrades; the common Latin/CJK/emoji domain can use bulk widths.
        for pattern in [
            "The café opens. 猫と犬。 e\u{301}",
            "👩\u{200d}💻🇫🇷🇺🇸☀\u{fe0f}",
            "لا ل\u{65f}\u{65e}أ",
            "\u{1a15}\u{1a17}\u{200d}\u{1a10}",
            "א\u{200d}ל",
            "\u{17d2}\u{1780}",
            "\u{16d68}\u{16d69}\u{16d6a}",
            "\u{a4f8}\u{a4fc}",
            "\u{10c32}\u{200d}\u{10c03}",
            "\u{2d4f}\u{2d7f}\u{2d3e}",
            "a\0\u{7f}\u{85}\r",
        ] {
            let text = format!(
                "{}\t{}\n{}",
                pattern.repeat(1000),
                pattern,
                pattern.repeat(400)
            );
            let index = CellIndex::from_text(&text);
            for at in text
                .char_indices()
                .map(|(at, _)| at)
                .step_by(157)
                .chain([text.len()])
            {
                let start = text[..at].rfind('\n').map_or(0, |at| at + 1);
                let end = text[at..].find('\n').map_or(text.len(), |end| at + end);
                assert_eq!(
                    index
                        .cell_at(at, |range| Ok(Cow::Borrowed(&text[range])))
                        .unwrap(),
                    scalar_to_cell(&text[start..end], text[start..at].chars().count()),
                    "pattern={pattern:?} at={at}"
                );
            }
        }
    }

    #[test]
    fn repairs_cross_long_regional_indicator_runs_and_single_graphemes() {
        // Odd RI insertion changes pairing across every block in the run. A
        // long extender cluster also crosses the builder's 64 KiB input chunks.
        for original in [
            format!("{}🇫\tabc", "🇫🇷".repeat(3000)),
            format!("e{}\tabc", "\u{301}".repeat(40_000)),
        ] {
            let mut builder = CellIndexBuilder::new();
            for ch in original.chars() {
                builder.push(ch.encode_utf8(&mut [0; 4]));
            }
            let mut index = builder.finish();
            let mut text = original.clone();
            for (range, insertion) in [(0..0, "🇦"), (0..4, ""), (0..1, "x")] {
                if !text.is_char_boundary(range.end) {
                    continue;
                }
                let splice = index.prepare_splice(
                    range.clone(),
                    |builder| {
                        builder.push(insertion);
                        Ok(())
                    },
                    |range| Ok(Cow::Borrowed(&text[range])),
                );
                index.apply_splice(splice);
                text.replace_range(range, insertion);
                // Check around block/chunk boundaries and at the final tab,
                // including scalar positions inside a multi-scalar grapheme.
                let positions: Vec<_> = text
                    .char_indices()
                    .map(|(at, _)| at)
                    .enumerate()
                    .filter(|(i, _)| i % 997 == 0)
                    .map(|(_, at)| at)
                    .chain([text.len() - 4, text.len() - 3, text.len()])
                    .collect();
                for at in positions {
                    assert_eq!(
                        index
                            .cell_at(at, |range| Ok(Cow::Borrowed(&text[range])))
                            .unwrap(),
                        scalar_to_cell(&text, text[..at].chars().count()),
                        "at={at}"
                    );
                }
            }
        }
    }

    #[test]
    fn local_splices_repair_unicode_joins_and_reset_cells_after_newlines() {
        for original in [
            "abcdef",
            "猫\te\u{301}👩\u{200d}💻",
            "🇫🇷🇺🇸🇯",
            "a\n猫",
            "\u{600}ab",
            "क्‍कxy",
        ] {
            for insertion in ["X", "\t", "\u{301}", "\u{200d}", "🇦", "\n", "👩"] {
                for at in original
                    .char_indices()
                    .map(|(at, _)| at)
                    .chain([original.len()])
                {
                    let mut text = original.to_owned();
                    let mut index = CellIndex::from_text(&text);
                    let splice = index.prepare_splice(
                        at..at,
                        |builder| {
                            builder.push(insertion);
                            Ok(())
                        },
                        |range| Ok(Cow::Borrowed(&text[range])),
                    );
                    index.apply_splice(splice);
                    text.insert_str(at, insertion);
                    check(&index, &text);
                    let splice = index.prepare_splice(
                        at..at + insertion.len(),
                        |_| Ok(()),
                        |range| Ok(Cow::Borrowed(&text[range])),
                    );
                    index.apply_splice(splice);
                    text.replace_range(at..at + insertion.len(), "");
                    check(&index, &text);
                }
            }
        }
    }
}
