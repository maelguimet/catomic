//! Bounded forward literal searches from an explicit byte position. A replacement
//! workflow supplies its wrap boundary and resumes after each handled candidate;
//! scans never restart at byte zero unless that workflow explicitly wraps.

use std::io;

use super::{literal::LiteralByteMatcher, SearchMatch};
use crate::buffer::Buffer;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ForwardMatch {
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) found: SearchMatch,
}

pub(crate) struct ForwardSearchTask {
    matcher: LiteralByteMatcher,
    start_byte: usize,
    stop_before: Option<usize>,
    query_bytes: usize,
    query_scalars: usize,
    invalid_query: bool,
}

impl ForwardSearchTask {
    pub(crate) fn new(query: &str, start_byte: usize, stop_before: Option<usize>) -> Self {
        Self {
            matcher: LiteralByteMatcher::new(query.as_bytes()),
            start_byte,
            stop_before,
            query_bytes: query.len(),
            query_scalars: query.chars().count(),
            invalid_query: query.is_empty() || query.contains('\n'),
        }
    }

    /// None means more bounded work remains. An empty vector means exhausted;
    /// otherwise the caller consumes a bounded snapshot and resumes explicitly.
    pub(crate) fn poll(
        &mut self,
        buffer: &dyn Buffer,
        budget: usize,
        max_matches: usize,
    ) -> io::Result<Option<Vec<ForwardMatch>>> {
        if self.invalid_query || self.stop_before.is_some_and(|stop| self.start_byte >= stop) {
            return Ok(Some(Vec::new()));
        }
        let source = buffer.piece_table_search().ok_or_else(|| {
            io::Error::other("buffer does not support bounded replacement search")
        })?;
        let mut remaining = budget;
        while remaining > 0 {
            let offset = self.start_byte + self.matcher.processed_bytes();
            // A candidate beginning just before the stop may end beyond it.
            let read_end = self
                .stop_before
                .map(|stop| stop.saturating_add(self.query_bytes - 1));
            if read_end.is_some_and(|end| offset >= end) {
                return Ok(Some(Vec::new()));
            }
            let bytes = read_end.map_or(remaining, |end| remaining.min(end - offset));
            let Some(segment) = source.text_segment(offset, bytes)? else {
                return Ok(Some(Vec::new()));
            };
            if segment.is_empty() {
                return Ok(Some(Vec::new()));
            }
            self.matcher
                .find_segment_matches_limited(segment.as_bytes(), max_matches.max(1));
            let mut matches = Vec::new();
            for &relative in self.matcher.candidates() {
                let start_byte = self.start_byte + relative;
                if self.stop_before.is_some_and(|stop| start_byte >= stop) {
                    break;
                }
                let start = source.cursor_for_byte_offset(start_byte)?;
                matches.push(ForwardMatch {
                    start_byte,
                    end_byte: start_byte + self.query_bytes,
                    found: SearchMatch {
                        start,
                        end_col: start.col + self.query_scalars,
                    },
                });
            }
            if !matches.is_empty() {
                return Ok(Some(matches));
            }
            let retained = self.matcher.retained_start(segment.as_bytes());
            self.matcher.commit_segment(segment.as_bytes(), retained);
            remaining = remaining.saturating_sub(segment.len());
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Cursor, PieceTable};

    #[test]
    fn starts_near_eof_and_includes_cross_boundary_and_overlapping_matches() {
        let prefix = "x".repeat(1024 * 1024);
        let mut buffer = PieceTable::from_text(&format!("{prefix}aaaaa猫"));
        buffer.set_cursor(Cursor {
            row: 0,
            col: prefix.len() + 2,
        });
        buffer.insert_char('a');
        let mut task = ForwardSearchTask::new("aaa", prefix.len(), Some(prefix.len() + 2));
        let found = loop {
            if let Some(found) = task.poll(&buffer, 2, 128).unwrap() {
                break found;
            }
        };
        assert_eq!(found[0].start_byte, prefix.len());
        assert_eq!(found[0].end_byte, prefix.len() + 3);
        let mut task = ForwardSearchTask::new("aaa", prefix.len() + 1, Some(prefix.len() + 2));
        let found = task.poll(&buffer, 64, 128).unwrap().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].start_byte, prefix.len() + 1);
    }

    #[test]
    fn polls_are_bounded_and_candidate_count_is_capped() {
        let buffer = PieceTable::from_text(&format!("{}aaaaa", "x".repeat(128 * 1024)));
        let mut task = ForwardSearchTask::new("a", 0, None);
        assert!(task.poll(&buffer, 64 * 1024, 2).unwrap().is_none());
        assert!(task.poll(&buffer, 64 * 1024, 2).unwrap().is_none());
        assert_eq!(task.poll(&buffer, 64 * 1024, 2).unwrap().unwrap().len(), 2);
    }
}
