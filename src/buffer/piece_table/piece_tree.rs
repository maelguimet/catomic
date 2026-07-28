//! Ordered piece descriptors with byte-count summaries.
//!
//! The AVL tree is indexed by in-order piece position. Each node caches its
//! subtree's descriptor count and logical byte length, so byte lookup and
//! local descriptor splices do not rebuild prefix sums or visit unrelated
//! pieces.

use std::ops::Range;

use super::types::Piece;

#[derive(Clone, Debug, Default)]
pub(crate) struct PieceTree {
    root: Option<Box<PieceNode>>,
}

#[derive(Clone, Debug)]
struct PieceNode {
    piece: Piece,
    left: Option<Box<PieceNode>>,
    right: Option<Box<PieceNode>>,
    height: usize,
    piece_count: usize,
    byte_len: usize,
}

impl PieceNode {
    fn new(piece: Piece) -> Self {
        Self {
            piece,
            left: None,
            right: None,
            height: 1,
            piece_count: 1,
            byte_len: piece.len,
        }
    }

    fn update_summary(&mut self) {
        self.height = 1 + node_height(&self.left).max(node_height(&self.right));
        self.piece_count = 1 + node_count(&self.left) + node_count(&self.right);
        self.byte_len = node_bytes(&self.left) + self.piece.len + node_bytes(&self.right);
    }
}

impl PieceTree {
    pub(crate) fn from_pieces(pieces: Vec<Piece>) -> Self {
        Self {
            root: build_balanced(&pieces),
        }
    }

    pub(crate) fn len(&self) -> usize {
        node_count(&self.root)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub(crate) fn byte_len(&self) -> usize {
        node_bytes(&self.root)
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.len() * std::mem::size_of::<PieceNode>()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&Piece> {
        get_node(&self.root, index)
    }

    pub(crate) fn set(&mut self, index: usize, piece: Piece) {
        set_node(&mut self.root, index, piece);
    }

    /// Locate the piece containing `offset`, choosing the following piece at a
    /// boundary and the final piece at document end.
    pub(crate) fn locate(&self, offset: usize) -> (usize, usize) {
        let Some(mut node) = self.root.as_deref() else {
            return (0, 0);
        };
        let mut remaining = offset.min(self.byte_len());
        let mut preceding = 0usize;
        loop {
            let left_bytes = node_bytes_ref(node.left.as_deref());
            let left_count = node_count_ref(node.left.as_deref());
            if remaining < left_bytes {
                node = node
                    .left
                    .as_deref()
                    .expect("left bytes require a left node");
                continue;
            }
            if remaining < left_bytes + node.piece.len {
                return (preceding + left_count, remaining - left_bytes);
            }
            remaining -= left_bytes + node.piece.len;
            preceding += left_count + 1;
            match node.right.as_deref() {
                Some(right) => node = right,
                None => return (preceding - 1, node.piece.len),
            }
        }
    }

    pub(crate) fn logical_start(&self, index: usize) -> usize {
        logical_start(&self.root, index.min(self.len()))
    }

    pub(crate) fn replace_range(&mut self, range: Range<usize>, replacement: Vec<Piece>) {
        debug_assert!(range.start <= range.end);
        debug_assert!(range.end <= self.len());
        let root = self.root.take();
        let (left, rest) = split(root, range.start);
        let (_, right) = split(rest, range.end - range.start);
        let middle = build_balanced(&replacement);
        self.root = concat(concat(left, middle), right);
    }

    pub(crate) fn collect_range(&self, range: Range<usize>) -> Vec<Piece> {
        let mut pieces = Vec::with_capacity(range.end.saturating_sub(range.start));
        let mut remaining = range.end.saturating_sub(range.start);
        self.for_each_from(range.start, |piece| {
            if remaining == 0 {
                return false;
            }
            pieces.push(*piece);
            remaining -= 1;
            remaining > 0
        });
        pieces
    }

    pub(crate) fn try_for_each<E>(
        &self,
        mut visit: impl FnMut(&Piece) -> Result<(), E>,
    ) -> Result<(), E> {
        try_for_each_node(&self.root, &mut visit)
    }

    pub(crate) fn for_each(&self, mut visit: impl FnMut(&Piece)) {
        for_each_node(&self.root, &mut visit);
    }

    pub(crate) fn for_each_mut(&mut self, mut visit: impl FnMut(&mut Piece)) {
        for_each_node_mut(&mut self.root, &mut visit);
    }

    pub(crate) fn try_for_each_from<E>(
        &self,
        index: usize,
        mut visit: impl FnMut(&Piece) -> Result<bool, E>,
    ) -> Result<(), E> {
        try_for_each_from_node(&self.root, index, &mut visit).map(|_| ())
    }

    fn for_each_from(&self, index: usize, mut visit: impl FnMut(&Piece) -> bool) {
        let _ = for_each_from_node(&self.root, index, &mut visit);
    }
}

fn node_height(node: &Option<Box<PieceNode>>) -> usize {
    node.as_deref().map_or(0, |node| node.height)
}

fn node_count(node: &Option<Box<PieceNode>>) -> usize {
    node.as_deref().map_or(0, |node| node.piece_count)
}

fn node_count_ref(node: Option<&PieceNode>) -> usize {
    node.map_or(0, |node| node.piece_count)
}

fn node_bytes(node: &Option<Box<PieceNode>>) -> usize {
    node.as_deref().map_or(0, |node| node.byte_len)
}

fn node_bytes_ref(node: Option<&PieceNode>) -> usize {
    node.map_or(0, |node| node.byte_len)
}

fn get_node(node: &Option<Box<PieceNode>>, index: usize) -> Option<&Piece> {
    let node = node.as_deref()?;
    let left_count = node_count(&node.left);
    if index < left_count {
        get_node(&node.left, index)
    } else if index == left_count {
        Some(&node.piece)
    } else {
        get_node(&node.right, index - left_count - 1)
    }
}

fn set_node(node: &mut Option<Box<PieceNode>>, index: usize, piece: Piece) {
    let Some(node) = node.as_mut() else {
        return;
    };
    let left_count = node_count(&node.left);
    if index < left_count {
        set_node(&mut node.left, index, piece);
    } else if index == left_count {
        node.piece = piece;
    } else {
        set_node(&mut node.right, index - left_count - 1, piece);
    }
    node.update_summary();
}

fn logical_start(node: &Option<Box<PieceNode>>, index: usize) -> usize {
    let Some(node) = node.as_deref() else {
        return 0;
    };
    let left_count = node_count(&node.left);
    if index < left_count {
        logical_start(&node.left, index)
    } else if index == left_count {
        node_bytes(&node.left)
    } else {
        node_bytes(&node.left) + node.piece.len + logical_start(&node.right, index - left_count - 1)
    }
}

fn build_balanced(pieces: &[Piece]) -> Option<Box<PieceNode>> {
    if pieces.is_empty() {
        return None;
    }
    let middle = pieces.len() / 2;
    let mut node = Box::new(PieceNode::new(pieces[middle]));
    node.left = build_balanced(&pieces[..middle]);
    node.right = build_balanced(&pieces[middle + 1..]);
    node.update_summary();
    Some(node)
}

fn split(
    root: Option<Box<PieceNode>>,
    left_count: usize,
) -> (Option<Box<PieceNode>>, Option<Box<PieceNode>>) {
    let Some(mut root) = root else {
        return (None, None);
    };
    let root_left_count = node_count(&root.left);
    if left_count <= root_left_count {
        let left = root.left.take();
        let right = root.right.take();
        let (before, middle) = split(left, left_count);
        root.update_summary();
        (before, Some(join(middle, root, right)))
    } else {
        let left = root.left.take();
        let right = root.right.take();
        let (middle, after) = split(right, left_count - root_left_count - 1);
        root.update_summary();
        (Some(join(left, root, middle)), after)
    }
}

fn concat(left: Option<Box<PieceNode>>, right: Option<Box<PieceNode>>) -> Option<Box<PieceNode>> {
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (Some(left), Some(right)) => {
            let (left, root) = take_max(left);
            Some(join(left, root, Some(right)))
        }
    }
}

fn take_max(mut root: Box<PieceNode>) -> (Option<Box<PieceNode>>, Box<PieceNode>) {
    match root.right.take() {
        None => {
            let left = root.left.take();
            root.update_summary();
            (left, root)
        }
        Some(right) => {
            let (new_right, max) = take_max(right);
            root.right = new_right;
            (Some(balance(root)), max)
        }
    }
}

fn join(
    left: Option<Box<PieceNode>>,
    mut root: Box<PieceNode>,
    right: Option<Box<PieceNode>>,
) -> Box<PieceNode> {
    if node_height(&left) > node_height(&right) + 1 {
        let mut left_root = left.expect("height proves left root exists");
        let left_right = left_root.right.take();
        left_root.right = Some(join(left_right, root, right));
        return balance(left_root);
    }
    if node_height(&right) > node_height(&left) + 1 {
        let mut right_root = right.expect("height proves right root exists");
        let right_left = right_root.left.take();
        right_root.left = Some(join(left, root, right_left));
        return balance(right_root);
    }
    root.left = left;
    root.right = right;
    root.update_summary();
    root
}

fn balance(mut root: Box<PieceNode>) -> Box<PieceNode> {
    root.update_summary();
    let left_height = node_height(&root.left);
    let right_height = node_height(&root.right);
    if left_height > right_height + 1 {
        if let Some(left) = root.left.as_ref() {
            if node_height(&left.right) > node_height(&left.left) {
                root.left = root.left.take().map(rotate_left);
            }
        }
        return rotate_right(root);
    }
    if right_height > left_height + 1 {
        if let Some(right) = root.right.as_ref() {
            if node_height(&right.left) > node_height(&right.right) {
                root.right = root.right.take().map(rotate_right);
            }
        }
        return rotate_left(root);
    }
    root
}

fn rotate_left(mut root: Box<PieceNode>) -> Box<PieceNode> {
    let mut pivot = root
        .right
        .take()
        .expect("left rotation requires right child");
    root.right = pivot.left.take();
    root.update_summary();
    pivot.left = Some(root);
    pivot.update_summary();
    pivot
}

fn rotate_right(mut root: Box<PieceNode>) -> Box<PieceNode> {
    let mut pivot = root
        .left
        .take()
        .expect("right rotation requires left child");
    root.left = pivot.right.take();
    root.update_summary();
    pivot.right = Some(root);
    pivot.update_summary();
    pivot
}

fn try_for_each_node<E>(
    node: &Option<Box<PieceNode>>,
    visit: &mut impl FnMut(&Piece) -> Result<(), E>,
) -> Result<(), E> {
    let Some(node) = node.as_deref() else {
        return Ok(());
    };
    try_for_each_node(&node.left, visit)?;
    visit(&node.piece)?;
    try_for_each_node(&node.right, visit)
}

fn for_each_node(node: &Option<Box<PieceNode>>, visit: &mut impl FnMut(&Piece)) {
    let Some(node) = node.as_deref() else {
        return;
    };
    for_each_node(&node.left, visit);
    visit(&node.piece);
    for_each_node(&node.right, visit);
}

fn for_each_node_mut(node: &mut Option<Box<PieceNode>>, visit: &mut impl FnMut(&mut Piece)) {
    let Some(node) = node.as_mut() else {
        return;
    };
    for_each_node_mut(&mut node.left, visit);
    visit(&mut node.piece);
    for_each_node_mut(&mut node.right, visit);
    node.update_summary();
}

fn for_each_from_node(
    node: &Option<Box<PieceNode>>,
    index: usize,
    visit: &mut impl FnMut(&Piece) -> bool,
) -> bool {
    let Some(node) = node.as_deref() else {
        return true;
    };
    let left_count = node_count(&node.left);
    if index < left_count && !for_each_from_node(&node.left, index, visit) {
        return false;
    }
    if index <= left_count && !visit(&node.piece) {
        return false;
    }
    let right_index = index.saturating_sub(left_count + 1);
    if index <= left_count || right_index < node_count(&node.right) {
        return for_each_from_node(&node.right, right_index, visit);
    }
    true
}

fn try_for_each_from_node<E>(
    node: &Option<Box<PieceNode>>,
    index: usize,
    visit: &mut impl FnMut(&Piece) -> Result<bool, E>,
) -> Result<bool, E> {
    let Some(node) = node.as_deref() else {
        return Ok(true);
    };
    let left_count = node_count(&node.left);
    if index < left_count && !try_for_each_from_node(&node.left, index, visit)? {
        return Ok(false);
    }
    if index <= left_count && !visit(&node.piece)? {
        return Ok(false);
    }
    let right_index = index.saturating_sub(left_count + 1);
    if index <= left_count || right_index < node_count(&node.right) {
        return try_for_each_from_node(&node.right, right_index, visit);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::piece_table::types::Source;

    #[test]
    fn randomized_splices_keep_order_summaries_and_exact_lookup() {
        let mut seed = 0x250_C0FFEE_u64;
        let mut next_piece_start = 0usize;
        let mut model = Vec::new();
        for len in [1, 3, 2, 4, 1] {
            model.push(piece(next_piece_start, len));
            next_piece_start += len;
        }
        let mut tree = PieceTree::from_pieces(model.clone());
        assert_matches_model(&tree, &model);

        for _ in 0..400 {
            seed = next_seed(seed);
            let start = (seed as usize) % (model.len() + 1);
            seed = next_seed(seed);
            let remove = (seed as usize) % (model.len() - start + 1);
            seed = next_seed(seed);
            let insert_count = (seed as usize) % 4;
            let mut inserted = Vec::with_capacity(insert_count);
            for _ in 0..insert_count {
                seed = next_seed(seed);
                let len = (seed as usize % 4) + 1;
                inserted.push(piece(next_piece_start, len));
                next_piece_start += len;
            }

            tree.replace_range(start..start + remove, inserted.clone());
            model.splice(start..start + remove, inserted);
            assert_matches_model(&tree, &model);
        }
    }

    fn piece(start: usize, len: usize) -> Piece {
        Piece {
            source: Source::Add,
            start,
            len,
        }
    }

    fn next_seed(seed: u64) -> u64 {
        seed.wrapping_mul(6364136223846793005).wrapping_add(1)
    }

    fn assert_matches_model(tree: &PieceTree, model: &[Piece]) {
        assert_eq!(tree.len(), model.len());
        assert_eq!(
            tree.collect_range(0..tree.len()),
            model,
            "in-order pieces drifted"
        );
        let total = model.iter().map(|piece| piece.len).sum::<usize>();
        assert_eq!(tree.byte_len(), total);

        let mut logical_start = 0usize;
        for (index, piece) in model.iter().enumerate() {
            assert_eq!(tree.get(index), Some(piece));
            assert_eq!(tree.logical_start(index), logical_start);
            for local in 0..piece.len {
                assert_eq!(tree.locate(logical_start + local), (index, local));
            }
            logical_start += piece.len;
        }
        if let Some(last) = model.last() {
            assert_eq!(tree.locate(total), (model.len() - 1, last.len));
        } else {
            assert_eq!(tree.locate(0), (0, 0));
        }
        assert_balanced(&tree.root);
    }

    fn assert_balanced(node: &Option<Box<PieceNode>>) -> (usize, usize, usize) {
        let Some(node) = node.as_deref() else {
            return (0, 0, 0);
        };
        let (left_height, left_count, left_bytes) = assert_balanced(&node.left);
        let (right_height, right_count, right_bytes) = assert_balanced(&node.right);
        assert!(left_height.abs_diff(right_height) <= 1);
        assert_eq!(node.height, 1 + left_height.max(right_height));
        assert_eq!(node.piece_count, 1 + left_count + right_count);
        assert_eq!(node.byte_len, left_bytes + node.piece.len + right_bytes);
        (node.height, node.piece_count, node.byte_len)
    }
}
