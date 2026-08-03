//! Sparse scalar/byte coordinates for immutable or append-only UTF-8 sources.
//!
//! Checkpoints are source-relative, so piece splits and logical edits never
//! invalidate them. ASCII immutable sources use direct byte arithmetic.

#[cfg(test)]
use std::cell::Cell;
use std::ops::Range;

pub(crate) const SCALAR_CHECKPOINT_INTERVAL: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScalarCheckpoint {
    scalar: usize,
    byte: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ScalarIndex {
    checkpoints: Vec<ScalarCheckpoint>,
    scalar_len: usize,
    byte_len: usize,
    is_ascii: bool,
    #[cfg(test)]
    visited_bytes: Cell<usize>,
}

impl ScalarIndex {
    pub(crate) fn for_immutable_text(text: &str) -> Self {
        if text.is_ascii() {
            return Self {
                checkpoints: vec![ScalarCheckpoint { scalar: 0, byte: 0 }],
                scalar_len: text.len(),
                byte_len: text.len(),
                is_ascii: true,
                #[cfg(test)]
                visited_bytes: Cell::new(0),
            };
        }
        Self::for_appendable_text(text)
    }

    /// Build coordinates for a source that will continue to grow. Unlike the
    /// immutable ASCII fast path, this retains sparse checkpoints so a later
    /// non-ASCII append cannot expose an unindexed ASCII prefix.
    pub(crate) fn for_appendable_text(text: &str) -> Self {
        let mut index = Self::empty_appendable();
        index.append(text);
        index
    }

    pub(crate) fn empty_appendable() -> Self {
        Self {
            checkpoints: vec![ScalarCheckpoint { scalar: 0, byte: 0 }],
            scalar_len: 0,
            byte_len: 0,
            is_ascii: true,
            #[cfg(test)]
            visited_bytes: Cell::new(0),
        }
    }

    pub(crate) fn append(&mut self, text: &str) {
        let base_byte = self.byte_len;
        let mut appended_is_ascii = true;
        for (relative_byte, ch) in text.char_indices() {
            appended_is_ascii &= ch.is_ascii();
            let next_scalar = self.scalar_len + 1;
            let next_byte = base_byte + relative_byte + ch.len_utf8();
            if next_scalar.is_multiple_of(SCALAR_CHECKPOINT_INTERVAL) {
                self.checkpoints.push(ScalarCheckpoint {
                    scalar: next_scalar,
                    byte: next_byte,
                });
            }
            self.scalar_len = next_scalar;
        }
        self.byte_len += text.len();
        self.is_ascii &= appended_is_ascii;
    }

    pub(crate) fn scalar_len(&self) -> usize {
        self.scalar_len
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.checkpoints.capacity() * std::mem::size_of::<ScalarCheckpoint>()
    }

    #[cfg(test)]
    pub(crate) fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    pub(crate) fn scalar_count(&self, text: &str, range: Range<usize>) -> usize {
        if self.is_ascii {
            return range.len();
        }
        self.scalar_at_byte(text, range.end) - self.scalar_at_byte(text, range.start)
    }

    pub(crate) fn byte_at_scalar_in(
        &self,
        text: &str,
        range: Range<usize>,
        scalar: usize,
    ) -> usize {
        if self.is_ascii {
            return range.start + scalar.min(range.len());
        }
        let range_start_scalar = self.scalar_at_byte(text, range.start);
        let range_end_scalar = self.scalar_at_byte(text, range.end);
        let target = range_start_scalar
            .saturating_add(scalar)
            .min(range_end_scalar);
        self.byte_at_scalar(text, target)
    }

    fn scalar_at_byte(&self, text: &str, byte: usize) -> usize {
        let byte = byte.min(self.byte_len);
        let index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.byte <= byte)
            .saturating_sub(1);
        let checkpoint = self.checkpoints[index];
        let tail = &text[checkpoint.byte..byte];
        self.record_visit(tail.len());
        checkpoint.scalar + tail.chars().count()
    }

    fn byte_at_scalar(&self, text: &str, scalar: usize) -> usize {
        let scalar = scalar.min(self.scalar_len);
        let index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.scalar <= scalar)
            .saturating_sub(1);
        let checkpoint = self.checkpoints[index];
        let remaining = scalar - checkpoint.scalar;
        if remaining == 0 {
            return checkpoint.byte;
        }
        let tail = &text[checkpoint.byte..];
        let relative = tail
            .char_indices()
            .nth(remaining)
            .map_or(tail.len(), |(offset, _)| offset);
        self.record_visit(relative);
        checkpoint.byte + relative
    }

    #[cfg(test)]
    fn record_visit(&self, bytes: usize) {
        self.visited_bytes
            .set(self.visited_bytes.get().saturating_add(bytes));
    }

    #[cfg(not(test))]
    fn record_visit(&self, _bytes: usize) {}

    #[cfg(test)]
    pub(crate) fn take_visited_bytes(&self) -> usize {
        self.visited_bytes.replace(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_coordinates_match_utf8_oracle_across_checkpoint_boundaries() {
        let alphabet = ['a', 'é', '猫', '🙂'];
        let text = (0..4097)
            .map(|index| alphabet[index % alphabet.len()])
            .collect::<String>();
        let mut byte_offsets = text
            .char_indices()
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>();
        byte_offsets.push(text.len());

        for index in [
            ScalarIndex::for_immutable_text(&text),
            ScalarIndex::for_appendable_text(&text),
        ] {
            for scalar in [0, 1, 1023, 1024, 1025, 2047, 2048, 4096, 4097] {
                assert_eq!(
                    index.byte_at_scalar_in(&text, 0..text.len(), scalar),
                    byte_offsets[scalar]
                );
            }
            for (start, end) in [
                (0, 1023),
                (1, 1024),
                (1023, 1025),
                (1024, 2049),
                (2048, 4097),
            ] {
                let range = byte_offsets[start]..byte_offsets[end];
                assert_eq!(index.scalar_count(&text, range.clone()), end - start);
                for scalar in [0, (end - start) / 2, end - start] {
                    assert_eq!(
                        index.byte_at_scalar_in(&text, range.clone(), scalar),
                        byte_offsets[start + scalar]
                    );
                }
            }
        }
    }

    #[test]
    fn randomized_ranges_match_char_indices_oracle() {
        let alphabet = ['a', 'é', '猫', '🙂'];
        let text = (0..8193)
            .map(|index| alphabet[(index * 7 + 3) % alphabet.len()])
            .collect::<String>();
        let index = ScalarIndex::for_immutable_text(&text);
        let mut byte_offsets = text
            .char_indices()
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>();
        byte_offsets.push(text.len());
        let mut seed = 0x255_C0FFEE_u64;

        for _ in 0..512 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let first = (seed as usize) % byte_offsets.len();
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let second = (seed as usize) % byte_offsets.len();
            let (start, end) = if first <= second {
                (first, second)
            } else {
                (second, first)
            };
            let range = byte_offsets[start]..byte_offsets[end];
            assert_eq!(index.scalar_count(&text, range.clone()), end - start);
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let relative = if start == end {
                0
            } else {
                (seed as usize) % (end - start + 1)
            };
            assert_eq!(
                index.byte_at_scalar_in(&text, range, relative),
                byte_offsets[start + relative]
            );
        }
    }
}
