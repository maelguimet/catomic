//! Edit-friendly line index over logical document bytes.
//!
//! Line byte spans live in bounded blocks. A deterministic implicit treap keeps
//! subtree row and byte totals, so edits update one block and a logarithmic
//! summary path instead of shifting every later absolute line start.
//!
//! Spans use `usize` deliberately: blocks are compact by line count, not by
//! narrowing byte coordinates. A single very long line therefore needs no
//! overflow side table and has no 16- or 32-bit representation ceiling.

use std::sync::Arc;

use crate::buffer::piece_table::file_original::FileOriginalMetadata;

const TARGET_BLOCK_LINES: usize = 128;
const MAX_BLOCK_LINES: usize = TARGET_BLOCK_LINES * 2;
const PRIORITY_SEED: u64 = 0x6a09_e667_f3bc_c909;
const PRIORITY_STEP: u64 = 0x9e37_79b9_7f4a_7c15;

type BlockTree = Option<Box<BlockNode>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineIndexWork {
    pub(crate) blocks_touched: usize,
    pub(crate) summary_nodes_updated: usize,
}

impl LineIndexWork {
    fn touch_blocks(&mut self, count: usize) {
        self.blocks_touched = self.blocks_touched.saturating_add(count);
    }

    fn update_summary(&mut self) {
        self.summary_nodes_updated = self.summary_nodes_updated.saturating_add(1);
    }
}

#[derive(Clone, Debug)]
struct LineBlock {
    /// Byte length of each logical line, including its terminating `\n` when
    /// present. The final span may be zero for an empty file or trailing LF.
    spans: Vec<usize>,
    total_bytes: usize,
}

impl LineBlock {
    fn new(spans: Vec<usize>) -> Self {
        debug_assert!(!spans.is_empty());
        debug_assert!(spans.len() <= MAX_BLOCK_LINES);
        let total_bytes = spans.iter().copied().sum();
        Self { spans, total_bytes }
    }

    fn line_start(&self, local_row: usize) -> usize {
        self.spans[..local_row].iter().copied().sum()
    }

    fn set_span(&mut self, local_row: usize, span: usize) {
        let old = self.spans[local_row];
        self.spans[local_row] = span;
        self.total_bytes = self.total_bytes - old + span;
    }

    fn replace_span(&mut self, local_row: usize, replacement: &[usize]) {
        self.spans
            .splice(local_row..local_row + 1, replacement.iter().copied());
        self.total_bytes = self.spans.iter().copied().sum();
    }

    fn remove_span(&mut self, local_row: usize) {
        let removed = self.spans.remove(local_row);
        self.total_bytes -= removed;
    }
}

#[derive(Clone, Debug)]
struct BlockNode {
    block: LineBlock,
    priority: u64,
    left: BlockTree,
    right: BlockTree,
    subtree_lines: usize,
    subtree_bytes: usize,
    subtree_blocks: usize,
}

impl BlockNode {
    fn new(block: LineBlock, priority: u64) -> Box<Self> {
        let subtree_lines = block.spans.len();
        let subtree_bytes = block.total_bytes;
        Box::new(Self {
            block,
            priority,
            left: None,
            right: None,
            subtree_lines,
            subtree_bytes,
            subtree_blocks: 1,
        })
    }
}

/// Block-local line coordinates with logarithmic row/byte summaries.
#[derive(Clone, Debug)]
pub(crate) struct LineIndex {
    root: BlockTree,
    priority_state: u64,
    work: LineIndexWork,
    /// Untouched descriptor-backed pages borrow their canonical compact
    /// boundaries. The first valid edit materializes the current block tree.
    file_metadata: Option<Arc<FileOriginalMetadata>>,
}

impl Default for LineIndex {
    fn default() -> Self {
        Self::from_text("")
    }
}

impl LineIndex {
    pub(crate) fn from_text(text: &str) -> Self {
        let mut spans = Vec::new();
        let mut line_start = 0usize;
        for (newline, _) in text.match_indices('\n') {
            let next_line = newline + 1;
            spans.push(next_line - line_start);
            line_start = next_line;
        }
        spans.push(text.len() - line_start);
        Self::from_spans(spans)
    }

    fn from_spans(spans: Vec<usize>) -> Self {
        let spans = if spans.is_empty() { vec![0] } else { spans };
        let mut priority_state = PRIORITY_SEED;
        let mut work = LineIndexWork::default();
        let mut root = None;
        for chunk in spans.chunks(TARGET_BLOCK_LINES) {
            let node = Some(BlockNode::new(
                LineBlock::new(chunk.to_vec()),
                next_priority(&mut priority_state),
            ));
            root = merge(root, node, &mut work);
        }
        Self {
            root,
            priority_state,
            work: LineIndexWork::default(),
            file_metadata: None,
        }
    }

    pub(crate) fn from_file_metadata(metadata: Arc<FileOriginalMetadata>) -> Self {
        Self {
            root: None,
            priority_state: PRIORITY_SEED,
            work: LineIndexWork::default(),
            file_metadata: Some(metadata),
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.file_metadata.as_ref().map_or_else(
            || tree_lines(&self.root).max(1),
            |metadata| metadata.line_count(),
        )
    }

    pub(crate) fn total_bytes(&self) -> usize {
        self.file_metadata
            .as_ref()
            .map_or_else(|| tree_bytes(&self.root), |metadata| metadata.logical_len)
    }

    pub(crate) fn line_start_byte(&self, row: usize) -> usize {
        if let Some(metadata) = &self.file_metadata {
            return metadata.logical_line_start(row);
        }
        let row = row.min(self.line_count().saturating_sub(1));
        self.locate_row(row)
            .map(|located| located.bytes_before + located.node.block.line_start(located.local_row))
            .unwrap_or(0)
    }

    /// Byte offset of line content end, excluding the terminating `\n`.
    pub(crate) fn line_end_byte(&self, row: usize) -> usize {
        if let Some(metadata) = &self.file_metadata {
            return metadata.logical_line_end(row);
        }
        let row = row.min(self.line_count().saturating_sub(1));
        let start = self.line_start_byte(row);
        let span = self.line_span(row).unwrap_or(0);
        if row + 1 < self.line_count() {
            start + span.saturating_sub(1)
        } else {
            start + span
        }
    }

    pub(crate) fn row_for_byte(&self, byte: usize) -> usize {
        if let Some(metadata) = &self.file_metadata {
            return metadata.row_for_logical_byte(byte);
        }
        let line_count = self.line_count();
        let total_bytes = self.total_bytes();
        if line_count <= 1 || byte >= total_bytes {
            return line_count.saturating_sub(1);
        }

        let mut node = self.root.as_deref();
        let mut rows_before = 0usize;
        let mut bytes_before = 0usize;
        while let Some(current) = node {
            let left_bytes = tree_bytes(&current.left);
            let left_lines = tree_lines(&current.left);
            if byte < bytes_before + left_bytes {
                node = current.left.as_deref();
                continue;
            }

            let block_start = bytes_before + left_bytes;
            let block_end = block_start + current.block.total_bytes;
            if byte < block_end {
                let mut local_start = block_start;
                for (local_row, span) in current.block.spans.iter().copied().enumerate() {
                    if byte < local_start + span {
                        return rows_before + left_lines + local_row;
                    }
                    local_start += span;
                }
            }

            rows_before += left_lines + current.block.spans.len();
            bytes_before = block_end;
            node = current.right.as_deref();
        }
        line_count.saturating_sub(1)
    }

    pub(crate) fn insert_bytes(&mut self, at_byte: usize, byte_len: usize) {
        if byte_len == 0 || at_byte > self.total_bytes() {
            return;
        }
        self.materialize_file_metadata();
        let row = self.row_for_byte(at_byte);
        let old_span = self.line_span(row).unwrap_or(0);
        self.set_line_span(row, old_span + byte_len);
    }

    pub(crate) fn delete_bytes(&mut self, at_byte: usize, byte_len: usize) {
        if byte_len == 0 || at_byte.saturating_add(byte_len) > self.total_bytes() {
            return;
        }
        let row = self.row_for_byte(at_byte);
        let old_span = self.line_span(row).unwrap_or(0);
        if byte_len > old_span {
            return;
        }
        self.materialize_file_metadata();
        self.set_line_span(row, old_span - byte_len);
    }

    pub(crate) fn insert_newline(&mut self, at_byte: usize) {
        if at_byte > self.total_bytes() {
            return;
        }
        self.materialize_file_metadata();
        let row = self.row_for_byte(at_byte);
        let line_start = self.line_start_byte(row);
        let old_span = self.line_span(row).unwrap_or(0);
        let local_byte = at_byte.saturating_sub(line_start).min(old_span);
        let replacement = [local_byte + 1, old_span - local_byte];
        let LineIndex {
            root,
            priority_state,
            work,
            ..
        } = self;
        *root = replace_one_span(root.take(), row, &replacement, priority_state, work);
    }

    pub(crate) fn delete_newline(&mut self, newline_byte: usize) -> bool {
        if newline_byte >= self.total_bytes() {
            return false;
        }
        let row = self.row_for_byte(newline_byte);
        if row + 1 >= self.line_count() || self.line_end_byte(row) != newline_byte {
            return false;
        }
        let first = self.line_span(row).unwrap_or(0);
        let second = self.line_span(row + 1).unwrap_or(0);
        self.materialize_file_metadata();
        self.set_line_span(row, first - 1 + second);
        let LineIndex { root, work, .. } = self;
        *root = remove_one_span(root.take(), row + 1, work);
        true
    }

    /// Replace logical bytes using newline offsets relative to the inserted
    /// sequence. Work is proportional to affected/inserted blocks plus tree
    /// summaries; no unchanged document bytes are inspected.
    pub(crate) fn replace_byte_range(
        &mut self,
        start: usize,
        end: usize,
        inserted_bytes: usize,
        inserted_newlines: &[usize],
    ) -> bool {
        if start > end || end > self.total_bytes() {
            return false;
        }
        if start == end && inserted_bytes == 0 {
            return false;
        }
        if inserted_newlines.windows(2).any(|pair| pair[0] >= pair[1])
            || inserted_newlines
                .last()
                .is_some_and(|newline| *newline >= inserted_bytes)
        {
            return false;
        }

        self.materialize_file_metadata();
        let start_row = self.row_for_byte(start);
        let end_row = self.row_for_byte(end);
        if start_row == end_row && inserted_newlines.is_empty() {
            let old_span = self.line_span(start_row).unwrap_or(0);
            let removed_bytes = end - start;
            self.set_line_span(
                start_row,
                old_span.saturating_sub(removed_bytes) + inserted_bytes,
            );
            return true;
        }

        let prefix_bytes = start - self.line_start_byte(start_row);
        let end_line_start = self.line_start_byte(end_row);
        let suffix_bytes = self
            .line_span(end_row)
            .unwrap_or(0)
            .saturating_sub(end - end_line_start);
        let mut replacement_spans = Vec::with_capacity(inserted_newlines.len() + 1);
        let mut inserted_line_start = 0usize;
        for (index, newline) in inserted_newlines.iter().copied().enumerate() {
            let span = newline + 1 - inserted_line_start;
            replacement_spans.push(if index == 0 {
                prefix_bytes + span
            } else {
                span
            });
            inserted_line_start = newline + 1;
        }
        replacement_spans.push(
            (if inserted_newlines.is_empty() {
                prefix_bytes
            } else {
                0
            }) + inserted_bytes
                - inserted_line_start
                + suffix_bytes,
        );

        let remove_lines = end_row - start_row + 1;
        let LineIndex {
            root,
            priority_state,
            work,
            ..
        } = self;
        let (left, tail) = split_at_row(root.take(), start_row, priority_state, work);
        let (removed, right) = split_at_row(tail, remove_lines, priority_state, work);
        work.touch_blocks(tree_blocks(&removed));
        let inserted = build_tree_from_spans(&replacement_spans, priority_state, work);
        *root = merge(merge(left, inserted, work), right, work);
        true
    }

    fn line_span(&self, row: usize) -> Option<usize> {
        if let Some(metadata) = &self.file_metadata {
            let row = row.min(metadata.line_count().saturating_sub(1));
            let start = metadata.logical_line_start(row);
            return Some(if row + 1 < metadata.line_count() {
                metadata.logical_line_start(row + 1).saturating_sub(start)
            } else {
                metadata.logical_len.saturating_sub(start)
            });
        }
        self.locate_row(row)
            .map(|located| located.node.block.spans[located.local_row])
    }

    fn locate_row(&self, row: usize) -> Option<LocatedRow<'_>> {
        let mut node = self.root.as_deref();
        let mut target = row;
        let mut bytes_before = 0usize;
        while let Some(current) = node {
            let left_lines = tree_lines(&current.left);
            let left_bytes = tree_bytes(&current.left);
            if target < left_lines {
                node = current.left.as_deref();
            } else if target < left_lines + current.block.spans.len() {
                return Some(LocatedRow {
                    node: current,
                    local_row: target - left_lines,
                    bytes_before: bytes_before + left_bytes,
                });
            } else {
                target -= left_lines + current.block.spans.len();
                bytes_before += left_bytes + current.block.total_bytes;
                node = current.right.as_deref();
            }
        }
        None
    }

    fn set_line_span(&mut self, row: usize, span: usize) {
        debug_assert!(self.file_metadata.is_none());
        set_one_span(&mut self.root, row, span, &mut self.work);
    }

    fn materialize_file_metadata(&mut self) {
        let Some(metadata) = self.file_metadata.take() else {
            return;
        };
        let owned = Self::from_spans(metadata.logical_line_spans());
        self.root = owned.root;
        self.priority_state = owned.priority_state;
    }

    #[cfg(test)]
    pub(crate) fn reset_work(&mut self) {
        self.work = LineIndexWork::default();
    }

    #[cfg(test)]
    pub(crate) fn work(&self) -> LineIndexWork {
        self.work
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        fn tree_retained_bytes(tree: &BlockTree) -> usize {
            let Some(node) = tree.as_deref() else {
                return 0;
            };
            std::mem::size_of::<BlockNode>()
                + node.block.spans.capacity() * std::mem::size_of::<usize>()
                + tree_retained_bytes(&node.left)
                + tree_retained_bytes(&node.right)
        }

        if self.file_metadata.is_some() {
            0
        } else {
            tree_retained_bytes(&self.root)
        }
    }

    #[cfg(test)]
    pub(crate) fn uses_shared_file_metadata(&self) -> bool {
        self.file_metadata.is_some()
    }

    #[cfg(test)]
    fn line_starts(&self) -> Vec<usize> {
        (0..self.line_count())
            .map(|row| self.line_start_byte(row))
            .collect()
    }
}

struct LocatedRow<'a> {
    node: &'a BlockNode,
    local_row: usize,
    bytes_before: usize,
}

fn tree_lines(tree: &BlockTree) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_lines)
}

fn tree_bytes(tree: &BlockTree) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_bytes)
}

fn tree_blocks(tree: &BlockTree) -> usize {
    tree.as_ref().map_or(0, |node| node.subtree_blocks)
}

fn recalculate(node: &mut BlockNode, work: &mut LineIndexWork) {
    node.subtree_lines = tree_lines(&node.left) + node.block.spans.len() + tree_lines(&node.right);
    node.subtree_bytes = tree_bytes(&node.left) + node.block.total_bytes + tree_bytes(&node.right);
    node.subtree_blocks = tree_blocks(&node.left) + 1 + tree_blocks(&node.right);
    work.update_summary();
}

fn next_priority(state: &mut u64) -> u64 {
    *state = state.wrapping_add(PRIORITY_STEP);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn merge(left: BlockTree, right: BlockTree, work: &mut LineIndexWork) -> BlockTree {
    match (left, right) {
        (None, tree) | (tree, None) => tree,
        (Some(mut left), Some(mut right)) => {
            if left.priority >= right.priority {
                left.right = merge(left.right.take(), Some(right), work);
                recalculate(&mut left, work);
                Some(left)
            } else {
                right.left = merge(Some(left), right.left.take(), work);
                recalculate(&mut right, work);
                Some(right)
            }
        }
    }
}

fn split_at_row(
    tree: BlockTree,
    row: usize,
    priority_state: &mut u64,
    work: &mut LineIndexWork,
) -> (BlockTree, BlockTree) {
    let Some(mut node) = tree else {
        return (None, None);
    };
    let left_lines = tree_lines(&node.left);
    let block_lines = node.block.spans.len();
    if row < left_lines {
        let (left, inner_right) = split_at_row(node.left.take(), row, priority_state, work);
        node.left = inner_right;
        recalculate(&mut node, work);
        return (left, Some(node));
    }
    if row > left_lines + block_lines {
        let (inner_left, right) = split_at_row(
            node.right.take(),
            row - left_lines - block_lines,
            priority_state,
            work,
        );
        node.right = inner_left;
        recalculate(&mut node, work);
        return (Some(node), right);
    }
    if row == left_lines {
        let left = node.left.take();
        recalculate(&mut node, work);
        return (left, Some(node));
    }
    if row == left_lines + block_lines {
        let right = node.right.take();
        recalculate(&mut node, work);
        return (Some(node), right);
    }

    work.touch_blocks(1);
    let local_row = row - left_lines;
    let right_spans = node.block.spans.split_off(local_row);
    node.block.total_bytes = node.block.spans.iter().copied().sum();
    let left_subtree = node.left.take();
    let right_subtree = node.right.take();
    let left_node = Some(BlockNode::new(node.block, next_priority(priority_state)));
    let right_node = Some(BlockNode::new(
        LineBlock::new(right_spans),
        next_priority(priority_state),
    ));
    (
        merge(left_subtree, left_node, work),
        merge(right_node, right_subtree, work),
    )
}

fn build_tree_from_spans(
    spans: &[usize],
    priority_state: &mut u64,
    work: &mut LineIndexWork,
) -> BlockTree {
    let mut tree = None;
    for chunk in spans.chunks(TARGET_BLOCK_LINES) {
        work.touch_blocks(1);
        let node = Some(BlockNode::new(
            LineBlock::new(chunk.to_vec()),
            next_priority(priority_state),
        ));
        tree = merge(tree, node, work);
    }
    tree
}

fn set_one_span(tree: &mut BlockTree, row: usize, span: usize, work: &mut LineIndexWork) -> bool {
    let Some(node) = tree.as_mut() else {
        return false;
    };
    let left_lines = tree_lines(&node.left);
    let changed = if row < left_lines {
        set_one_span(&mut node.left, row, span, work)
    } else if row < left_lines + node.block.spans.len() {
        work.touch_blocks(1);
        node.block.set_span(row - left_lines, span);
        true
    } else {
        set_one_span(
            &mut node.right,
            row - left_lines - node.block.spans.len(),
            span,
            work,
        )
    };
    if changed {
        recalculate(node, work);
    }
    changed
}

fn replace_one_span(
    tree: BlockTree,
    row: usize,
    replacement: &[usize],
    priority_state: &mut u64,
    work: &mut LineIndexWork,
) -> BlockTree {
    let mut node = tree?;
    let left_lines = tree_lines(&node.left);
    if row < left_lines {
        node.left = replace_one_span(node.left.take(), row, replacement, priority_state, work);
        recalculate(&mut node, work);
        return Some(node);
    }
    if row >= left_lines + node.block.spans.len() {
        node.right = replace_one_span(
            node.right.take(),
            row - left_lines - node.block.spans.len(),
            replacement,
            priority_state,
            work,
        );
        recalculate(&mut node, work);
        return Some(node);
    }

    work.touch_blocks(1);
    node.block.replace_span(row - left_lines, replacement);
    if node.block.spans.len() <= MAX_BLOCK_LINES {
        recalculate(&mut node, work);
        return Some(node);
    }

    let right_spans = node.block.spans.split_off(TARGET_BLOCK_LINES);
    node.block.total_bytes = node.block.spans.iter().copied().sum();
    let left_subtree = node.left.take();
    let right_subtree = node.right.take();
    let left_node = Some(BlockNode::new(node.block, next_priority(priority_state)));
    let right_node = Some(BlockNode::new(
        LineBlock::new(right_spans),
        next_priority(priority_state),
    ));
    merge(
        merge(left_subtree, left_node, work),
        merge(right_node, right_subtree, work),
        work,
    )
}

fn remove_one_span(tree: BlockTree, row: usize, work: &mut LineIndexWork) -> BlockTree {
    let mut node = tree?;
    let left_lines = tree_lines(&node.left);
    if row < left_lines {
        node.left = remove_one_span(node.left.take(), row, work);
        recalculate(&mut node, work);
        return Some(node);
    }
    if row >= left_lines + node.block.spans.len() {
        node.right = remove_one_span(
            node.right.take(),
            row - left_lines - node.block.spans.len(),
            work,
        );
        recalculate(&mut node, work);
        return Some(node);
    }

    work.touch_blocks(1);
    node.block.remove_span(row - left_lines);
    if node.block.spans.is_empty() {
        merge(node.left.take(), node.right.take(), work)
    } else {
        recalculate(&mut node, work);
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::buffer::piece_table::file_original::FileOriginalMetadata;

    use super::{LineIndex, MAX_BLOCK_LINES};

    fn shared_crlf_index() -> LineIndex {
        let lines = MAX_BLOCK_LINES * 3;
        let range_start = 100usize;
        let line_starts = (0..=lines)
            .map(|row| range_start + row * 3)
            .collect::<Vec<_>>();
        let crlf_offsets = (0..lines)
            .map(|row| range_start + row * 3 + 1)
            .collect::<Vec<_>>();
        let metadata = FileOriginalMetadata::from_scan(
            range_start,
            range_start + lines * 3,
            lines * 2,
            line_starts,
            crlf_offsets,
            [vec![1; lines], vec![0]].concat(),
            vec![true; lines + 1],
            Vec::new(),
            vec![0; lines + 2],
        );
        LineIndex::from_file_metadata(Arc::new(metadata))
    }

    #[test]
    fn from_text_empty_has_single_start() {
        let index = LineIndex::from_text("");

        assert_eq!(index.line_starts(), vec![0]);
        assert_eq!(index.total_bytes(), 0);
        assert_eq!(index.line_count(), 1);
    }

    #[test]
    fn from_text_records_lf_line_starts() {
        let index = LineIndex::from_text("one\ntwo\n");

        assert_eq!(index.line_starts(), vec![0, 4, 8]);
        assert_eq!(index.total_bytes(), 8);
        assert_eq!(index.line_count(), 3);
    }

    #[test]
    fn from_text_uses_byte_offsets_for_multibyte_content() {
        let index = LineIndex::from_text("é\n猫\nx");

        assert_eq!(index.line_starts(), vec![0, 3, 7]);
        assert_eq!(index.total_bytes(), 8);
        assert_eq!(index.line_start_byte(2), 7);
        assert_eq!(index.line_end_byte(0), 2);
        assert_eq!(index.row_for_byte(2), 0);
        assert_eq!(index.row_for_byte(3), 1);
    }

    #[test]
    fn simple_edits_touch_one_block_and_logarithmic_summaries() {
        let text = "x\n".repeat(100_000);
        let mut top = LineIndex::from_text(&text);
        let mut bottom = top.clone();

        top.reset_work();
        top.insert_bytes(0, 1);
        let top_work = top.work();

        bottom.reset_work();
        bottom.insert_bytes(bottom.total_bytes(), 1);
        let bottom_work = bottom.work();

        assert_eq!(top_work.blocks_touched, 1);
        assert_eq!(bottom_work.blocks_touched, 1);
        assert!(top_work.summary_nodes_updated < 64, "{top_work:?}");
        assert!(bottom_work.summary_nodes_updated < 64, "{bottom_work:?}");
        assert_eq!(top.line_count(), bottom.line_count());
        assert_eq!(top.total_bytes(), bottom.total_bytes());
    }

    #[test]
    fn valid_mutations_materialize_shared_metadata_into_local_blocks() {
        let mut cases = [
            shared_crlf_index(),
            shared_crlf_index(),
            shared_crlf_index(),
            shared_crlf_index(),
            shared_crlf_index(),
        ];

        cases[0].insert_bytes(0, 1);
        cases[1].delete_bytes(0, 1);
        cases[2].insert_newline(1);
        assert!(cases[3].delete_newline(1));
        assert!(cases[4].replace_byte_range(0, 1, 2, &[0]));

        for index in &cases {
            assert!(!index.uses_shared_file_metadata());
            let work = index.work();
            assert!(work.blocks_touched <= 4, "{work:?}");
            assert!(work.summary_nodes_updated < 64, "{work:?}");
            assert!(index.retained_bytes() > 0);
        }

        cases[0].reset_work();
        let end = cases[0].total_bytes();
        cases[0].insert_bytes(end, 1);
        assert_eq!(cases[0].work().blocks_touched, 1);
        assert!(cases[0].work().summary_nodes_updated < 64);
    }

    #[test]
    fn invalid_mutations_do_not_materialize_shared_metadata() {
        let mut index = shared_crlf_index();
        let total = index.total_bytes();

        index.insert_bytes(0, 0);
        index.insert_bytes(total + 1, 1);
        index.delete_bytes(total, 2);
        assert!(!index.delete_newline(0));
        assert!(!index.replace_byte_range(total + 1, total + 1, 1, &[]));

        assert!(index.uses_shared_file_metadata());
        assert_eq!(index.retained_bytes(), 0);
    }

    #[test]
    fn newline_edits_split_blocks_without_shifting_the_tail() {
        let text = "a\n".repeat(MAX_BLOCK_LINES * 3);
        let mut index = LineIndex::from_text(&text);
        let original_lines = index.line_count();

        index.reset_work();
        index.insert_newline(1);
        let insert_work = index.work();
        assert_eq!(index.line_count(), original_lines + 1);
        assert!(insert_work.blocks_touched <= 2, "{insert_work:?}");
        assert!(insert_work.summary_nodes_updated < 64, "{insert_work:?}");

        index.reset_work();
        assert!(index.delete_newline(1));
        let delete_work = index.work();
        assert_eq!(index.line_count(), original_lines);
        assert!(delete_work.blocks_touched <= 2, "{delete_work:?}");
        assert!(delete_work.summary_nodes_updated < 64, "{delete_work:?}");
        assert_eq!(index.line_starts()[..4], [0, 2, 4, 6]);
    }

    #[test]
    fn replace_range_preserves_trailing_newline_and_long_utf8_lines() {
        let long = "猫".repeat(70_000);
        let text = format!("{long}\n🙂\n");
        let mut index = LineIndex::from_text(&text);
        let first_end = long.len();

        assert_eq!(index.line_end_byte(0), first_end);
        assert_eq!(index.line_start_byte(1), first_end + 1);
        assert_eq!(index.line_start_byte(2), text.len());

        assert!(index.replace_byte_range(first_end, first_end + 1, 4, &[]));
        assert_eq!(index.line_count(), 2);
        assert_eq!(index.line_start_byte(1), first_end + 4 + "🙂".len() + 1);
        assert_eq!(index.line_end_byte(1), index.total_bytes());
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn platform_sized_spans_exceed_u32_without_an_overflow_table() {
        let huge = u32::MAX as usize + 17;
        let index = LineIndex::from_spans(vec![huge + 1, 3, 0]);

        assert_eq!(index.line_start_byte(1), huge + 1);
        assert_eq!(index.line_start_byte(2), huge + 4);
        assert_eq!(index.row_for_byte(huge), 0);
        assert_eq!(index.row_for_byte(huge + 1), 1);
        assert_eq!(index.total_bytes(), huge + 4);
    }
}
