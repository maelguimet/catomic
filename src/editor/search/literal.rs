//! Bounded literal matching over a segmented logical byte stream.
//!
//! The matcher owns one reusable `memmem::Finder` per query. Callers retain
//! coordinate and I/O policy; this type only joins adjacent UTF-8 byte slices
//! with the minimum suffix needed to find cross-segment matches.

use memchr::memmem::Finder;

pub(super) struct LiteralByteMatcher {
    finder: Finder<'static>,
    query_len: usize,
    processed_bytes: usize,
    overlap: Vec<u8>,
    boundary: Vec<u8>,
    candidates: Vec<usize>,
}

impl LiteralByteMatcher {
    pub(super) fn new(query: &[u8]) -> Self {
        Self {
            finder: Finder::new(query).into_owned(),
            query_len: query.len(),
            processed_bytes: 0,
            overlap: Vec::with_capacity(query.len().saturating_sub(1)),
            boundary: Vec::with_capacity(query.len().saturating_mul(2).saturating_sub(2)),
            candidates: Vec::new(),
        }
    }

    pub(super) fn processed_bytes(&self) -> usize {
        self.processed_bytes
    }

    pub(super) fn overlap(&self) -> &[u8] {
        &self.overlap
    }

    pub(super) fn overlap_start(&self) -> usize {
        self.processed_bytes.saturating_sub(self.overlap.len())
    }

    pub(super) fn candidates(&self) -> &[usize] {
        &self.candidates
    }

    /// Find overlapping matches introduced by `segment`, optionally stopping at
    /// the first candidate at or beyond a caller-selected byte offset. The stream
    /// state is committed separately so descriptor callers can map a candidate in
    /// the old overlap before that suffix is replaced.
    pub(super) fn find_segment_matches(&mut self, segment: &[u8], stop_at_or_after: Option<usize>) {
        self.candidates.clear();
        let overlap_len = self.overlap.len();
        if overlap_len > 0 {
            self.boundary.clear();
            self.boundary.extend_from_slice(&self.overlap);
            self.boundary
                .extend_from_slice(&segment[..segment.len().min(self.query_len.saturating_sub(1))]);
            let boundary_start = self.processed_bytes - overlap_len;
            let stopped =
                find_overlapping(&self.finder, self.query_len, &self.boundary, |relative| {
                    if relative < overlap_len && relative + self.query_len > overlap_len {
                        let offset = boundary_start + relative;
                        self.candidates.push(offset);
                        return stop_at_or_after.is_some_and(|limit| offset >= limit);
                    }
                    false
                });
            if stopped {
                return;
            }
        }

        find_overlapping(&self.finder, self.query_len, segment, |relative| {
            let offset = self.processed_bytes + relative;
            self.candidates.push(offset);
            stop_at_or_after.is_some_and(|limit| offset >= limit)
        });
    }

    /// Return the scalar-aligned logical start of the suffix retained after
    /// `segment`. Valid UTF-8 matches never start on a continuation byte, so
    /// aligning forward cannot discard a possible cross-boundary match.
    pub(super) fn retained_start(&self, segment: &[u8]) -> usize {
        let stream_end = self.processed_bytes.saturating_add(segment.len());
        let mut start = stream_end
            .saturating_sub(self.query_len.saturating_sub(1))
            .max(self.overlap_start());
        while start < stream_end && is_utf8_continuation(self.byte_at(segment, start)) {
            start += 1;
        }
        start
    }

    pub(super) fn commit_segment(&mut self, segment: &[u8], retained_start: usize) {
        let old_overlap_start = self.overlap_start();
        let segment_start = self.processed_bytes;
        let stream_end = segment_start.saturating_add(segment.len());
        debug_assert!(retained_start >= old_overlap_start);
        debug_assert!(retained_start <= stream_end);

        if retained_start >= segment_start {
            let relative = retained_start - segment_start;
            self.overlap.clear();
            self.overlap.extend_from_slice(&segment[relative..]);
        } else {
            let relative = retained_start - old_overlap_start;
            self.overlap.drain(..relative);
            self.overlap.extend_from_slice(segment);
        }
        self.processed_bytes = stream_end;
        debug_assert!(self.overlap.len() <= self.query_len.saturating_sub(1));
    }

    fn byte_at(&self, segment: &[u8], logical_offset: usize) -> u8 {
        if logical_offset >= self.processed_bytes {
            segment[logical_offset - self.processed_bytes]
        } else {
            self.overlap[logical_offset - self.overlap_start()]
        }
    }

    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        self.overlap.len()
    }
}

fn find_overlapping(
    finder: &Finder<'_>,
    query_len: usize,
    haystack: &[u8],
    mut found: impl FnMut(usize) -> bool,
) -> bool {
    let mut search_start = 0usize;
    while search_start.saturating_add(query_len) <= haystack.len() {
        let Some(relative) = finder.find(&haystack[search_start..]) else {
            break;
        };
        let offset = search_start + relative;
        if found(offset) {
            return true;
        }
        search_start = offset + 1;
    }
    false
}

fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

#[cfg(test)]
mod tests {
    use super::LiteralByteMatcher;

    fn segmented_matches(query: &str, segments: &[&str]) -> (Vec<usize>, usize) {
        let mut matcher = LiteralByteMatcher::new(query.as_bytes());
        let mut matches = Vec::new();
        for segment in segments {
            matcher.find_segment_matches(segment.as_bytes(), None);
            matches.extend_from_slice(matcher.candidates());
            let retained_start = matcher.retained_start(segment.as_bytes());
            matcher.commit_segment(segment.as_bytes(), retained_start);
            assert!(matcher.retained_bytes() <= query.len().saturating_sub(1));
        }
        (matches, matcher.retained_bytes())
    }

    #[test]
    fn segmented_matching_preserves_every_overlapping_occurrence() {
        for segments in [
            vec!["aaaaa"],
            vec!["a", "a", "a", "a", "a"],
            vec!["aa", "a", "aa"],
        ] {
            let (matches, retained) = segmented_matches("aaa", &segments);
            assert_eq!(matches, [0, 1, 2]);
            assert!(retained <= 2);
        }
    }

    #[test]
    fn segmented_matching_keeps_only_a_scalar_aligned_query_suffix() {
        let segments = ["start ", "👩", "🏽", "\u{200d}", "💻", " end"];
        let query = "👩🏽\u{200d}💻";
        let (matches, retained) = segmented_matches(query, &segments);
        assert_eq!(matches, ["start ".len()]);
        assert!(retained < query.len());
    }

    #[test]
    fn query_longer_than_each_segment_crosses_many_boundaries_once() {
        let (matches, _) = segmented_matches("abcdefgh", &["ab", "cd", "ef", "gh", "x"]);
        assert_eq!(matches, [0]);
    }

    #[test]
    fn matching_can_stop_after_the_first_selectable_candidate() {
        let mut matcher = LiteralByteMatcher::new(b"aaa");
        matcher.find_segment_matches(b"aaaaa", Some(1));
        assert_eq!(matcher.candidates(), [0, 1]);
    }
}
