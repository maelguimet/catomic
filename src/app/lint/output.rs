//! Purpose: parse source locations and raw messages from explicit linter output.
//! Owns: common file:line:col:message parsing and path resolution.
//! Must not: run tools, access App/terminal state, scan projects, mutate files, or network.
//! Invariants: coordinates are retained as positive user-facing 1-based values.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedFinding {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

pub(crate) fn parse_common_output(output: &str, root: &Path) -> Vec<ParsedFinding> {
    output
        .lines()
        .filter_map(|line| parse_common_line(line, root))
        .collect()
}

fn parse_common_line(line: &str, root: &Path) -> Option<ParsedFinding> {
    let mut fields = line.splitn(4, ':');
    let file = fields.next()?.trim();
    let line = fields.next()?.trim().parse::<usize>().ok()?;
    let col = fields.next()?.trim().parse::<usize>().ok()?;
    let message = fields.next()?.trim();
    if file.is_empty() || line == 0 || col == 0 || message.is_empty() {
        return None;
    }
    let file = PathBuf::from(file);
    Some(ParsedFinding {
        file: if file.is_absolute() {
            file
        } else {
            root.join(file)
        },
        line,
        col,
        message: message.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_relative_and_absolute_findings() {
        let root = PathBuf::from("/work/project");
        let output = concat!(
            "src/main.rs:12:7: error: broken thing\n",
            "/tmp/other.py:3:2: warning: suspicious thing\n",
        );

        let parsed = parse_common_output(output, &root);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].file, root.join("src/main.rs"));
        assert_eq!((parsed[0].line, parsed[0].col), (12, 7));
        assert_eq!(parsed[0].message, "error: broken thing");
        assert_eq!(parsed[1].file, PathBuf::from("/tmp/other.py"));
        assert_eq!(parsed[1].message, "warning: suspicious thing");
    }

    #[test]
    fn skips_malformed_and_zero_coordinate_lines() {
        let output = concat!(
            "noise\n",
            "file.rs:x:2: nope\n",
            "file.rs:2:0: nope\n",
            "file.rs:4:5: info: useful note\n",
        );

        let parsed = parse_common_output(output, std::path::Path::new("/root"));

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].message, "info: useful note");
    }
}
