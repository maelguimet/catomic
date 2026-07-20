//! Purpose: this file must parse and preview safe single-file unified patches.
//! Owns: unified-diff validation, hunk representation, and context-checked preview.
//! Must not: mutate buffers, read files, accept multiple files, or bypass confirmation.
//! Invariants: hunk counts match their bodies; preview fails on stale source context.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub old_path: String,
    pub new_path: String,
    hunks: Vec<Hunk>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    lines: Vec<HunkLine>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchVisualization {
    /// Zero-based half-open logical line ranges containing model-added/replaced text.
    pub added_line_ranges: Vec<(usize, usize)>,
    /// Zero-based proposed-document lines where removed text formerly appeared.
    pub deleted_at_lines: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HunkLine {
    kind: HunkLineKind,
    text: String,
    no_newline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HunkLineKind {
    Context,
    Remove,
    Add,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    NoPatch,
    MissingNewPath,
    MultipleFiles,
    MalformedHunkHeader { line: usize },
    InvalidHunkLine { line: usize },
    CountMismatch { line: usize },
    MisplacedNoNewlineMarker { line: usize },
    UnexpectedPath,
    OverlappingHunks,
    SourceOutOfBounds,
    SourceMismatch { line: usize },
}

impl Patch {
    pub fn parse(text: &str) -> Result<Self, PatchError> {
        let lines: Vec<&str> = text.lines().collect();
        let old_header = lines
            .windows(2)
            .position(|pair| pair[0].starts_with("--- ") && pair[1].starts_with("+++ "))
            .ok_or(PatchError::NoPatch)?;
        let new_path = lines
            .get(old_header + 1)
            .and_then(|line| line.strip_prefix("+++ "))
            .ok_or(PatchError::MissingNewPath)?;
        let old_path = lines[old_header].trim_start_matches("--- ");
        let mut cursor = old_header + 2;
        let mut hunks = Vec::new();

        while cursor < lines.len() {
            if lines[cursor].starts_with("--- ") || lines[cursor].starts_with("diff --git ") {
                return Err(PatchError::MultipleFiles);
            }
            if !lines[cursor].starts_with("@@ ") {
                cursor += 1;
                continue;
            }
            let (hunk, next) = parse_hunk(&lines, cursor)?;
            hunks.push(hunk);
            cursor = next;
        }
        if hunks.is_empty() {
            return Err(PatchError::NoPatch);
        }
        Ok(Self {
            old_path: parse_header_path(old_path),
            new_path: parse_header_path(new_path),
            hunks,
        })
    }

    pub fn apply_preview(&self, current: &str) -> Result<String, PatchError> {
        let trailing_newline = current.ends_with('\n');
        let mut source: Vec<&str> = current.split('\n').collect();
        if trailing_newline {
            source.pop();
        }
        if current.is_empty() {
            source.clear();
        }

        let mut output = Vec::new();
        let mut source_cursor = 0;
        let mut result_trailing_newline = trailing_newline;
        for hunk in &self.hunks {
            let hunk_start = if hunk.old_count == 0 {
                hunk.old_start
            } else {
                hunk.old_start - 1
            };
            if hunk_start < source_cursor {
                return Err(PatchError::OverlappingHunks);
            }
            if hunk_start > source.len() {
                return Err(PatchError::SourceOutOfBounds);
            }
            output.extend(
                source[source_cursor..hunk_start]
                    .iter()
                    .map(|line| (*line).to_string()),
            );
            source_cursor = hunk_start;

            for line in &hunk.lines {
                match line.kind {
                    HunkLineKind::Context | HunkLineKind::Remove => {
                        let source_line = source
                            .get(source_cursor)
                            .ok_or(PatchError::SourceOutOfBounds)?;
                        if *source_line != line.text {
                            return Err(PatchError::SourceMismatch {
                                line: source_cursor,
                            });
                        }
                        source_cursor += 1;
                        if line.kind == HunkLineKind::Context {
                            output.push(line.text.clone());
                        }
                    }
                    HunkLineKind::Add => output.push(line.text.clone()),
                }
            }
            if source_cursor == source.len() {
                result_trailing_newline = hunk
                    .lines
                    .iter()
                    .rev()
                    .find(|line| line.kind != HunkLineKind::Remove)
                    .is_some_and(|line| !line.no_newline);
            }
        }
        output.extend(
            source[source_cursor..]
                .iter()
                .map(|line| (*line).to_string()),
        );

        let mut preview = output.join("\n");
        if result_trailing_newline && !output.is_empty() {
            preview.push('\n');
        }
        Ok(preview)
    }

    pub fn validate_target(&self, expected_path: &str) -> Result<(), PatchError> {
        let expected = expected_path.strip_prefix("./").unwrap_or(expected_path);
        let targets_expected = |path: &str| path == "/dev/null" || path == expected;
        if targets_expected(&self.old_path)
            && targets_expected(&self.new_path)
            && (self.old_path == expected || self.new_path == expected)
        {
            Ok(())
        } else {
            Err(PatchError::UnexpectedPath)
        }
    }

    pub fn visualization(&self) -> PatchVisualization {
        let mut visualization = PatchVisualization::default();
        for hunk in &self.hunks {
            let mut new_line = hunk.new_start.saturating_sub(1);
            let mut open_added = None;
            for line in &hunk.lines {
                if line.kind != HunkLineKind::Add {
                    if let Some(start) = open_added.take() {
                        visualization.added_line_ranges.push((start, new_line));
                    }
                }
                match line.kind {
                    HunkLineKind::Context => {
                        new_line += 1;
                    }
                    HunkLineKind::Remove => {
                        visualization.deleted_at_lines.push(new_line);
                    }
                    HunkLineKind::Add => {
                        open_added.get_or_insert(new_line);
                        new_line += 1;
                    }
                }
            }
            if let Some(start) = open_added {
                visualization.added_line_ranges.push((start, new_line));
            }
        }
        visualization.deleted_at_lines.sort_unstable();
        visualization.deleted_at_lines.dedup();
        visualization
    }
}

fn parse_hunk(lines: &[&str], header_line: usize) -> Result<(Hunk, usize), PatchError> {
    let (old_start, old_count, new_start, new_count) = parse_hunk_header(lines[header_line])
        .ok_or(PatchError::MalformedHunkHeader { line: header_line })?;
    let mut body: Vec<HunkLine> = Vec::new();
    let mut cursor = header_line + 1;
    while cursor < lines.len() && !lines[cursor].starts_with("@@ ") {
        if lines[cursor].starts_with("--- ")
            || lines[cursor].starts_with("diff --git ")
            || lines[cursor].starts_with("```")
        {
            break;
        }
        if lines[cursor] == "\\ No newline at end of file" {
            let previous = body
                .last_mut()
                .ok_or(PatchError::MisplacedNoNewlineMarker { line: cursor })?;
            previous.no_newline = true;
            cursor += 1;
            continue;
        }
        let (kind, text) = match lines[cursor].as_bytes().first() {
            Some(b' ') => (HunkLineKind::Context, &lines[cursor][1..]),
            Some(b'-') => (HunkLineKind::Remove, &lines[cursor][1..]),
            Some(b'+') => (HunkLineKind::Add, &lines[cursor][1..]),
            _ => return Err(PatchError::InvalidHunkLine { line: cursor }),
        };
        body.push(HunkLine {
            kind,
            text: text.to_string(),
            no_newline: false,
        });
        cursor += 1;
    }
    let actual_old = body
        .iter()
        .filter(|line| line.kind != HunkLineKind::Add)
        .count();
    let actual_new = body
        .iter()
        .filter(|line| line.kind != HunkLineKind::Remove)
        .count();
    if actual_old != old_count || actual_new != new_count {
        return Err(PatchError::CountMismatch { line: header_line });
    }
    Ok((
        Hunk {
            old_start,
            old_count,
            new_start,
            lines: body,
        },
        cursor,
    ))
}

fn parse_hunk_header(header: &str) -> Option<(usize, usize, usize, usize)> {
    let (ranges, _) = header.strip_prefix("@@ ")?.split_once(" @@")?;
    let mut parts = ranges.split_whitespace();
    let (old_start, old_count) = parse_range(parts.next()?, '-')?;
    let (new_start, new_count) = parse_range(parts.next()?, '+')?;
    parts
        .next()
        .is_none()
        .then_some((old_start, old_count, new_start, new_count))
}

fn parse_range(range: &str, prefix: char) -> Option<(usize, usize)> {
    let range = range.strip_prefix(prefix)?;
    let mut numbers = range.split(',');
    let start = numbers.next()?.parse().ok()?;
    let count = numbers.next().map_or(Some(1), |value| value.parse().ok())?;
    (numbers.next().is_none() && (start != 0 || count == 0)).then_some((start, count))
}

fn parse_header_path(header: &str) -> String {
    let path = header.split('\t').next().unwrap_or(header);
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod tests;
