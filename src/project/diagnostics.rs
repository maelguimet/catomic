//! Purpose: represent and parse Project-mode diagnostics from explicit tool output.
//! Owns: common file:line:col:message parsing, path resolution, and severity classification.
//! Must not: run tools, access App/terminal state, scan projects, mutate files, or network.
//! Invariants: coordinates are retained as positive user-facing 1-based values.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub severity: Severity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Collection of diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }
}

pub(crate) fn parse_common_output(output: &str, root: &Path) -> Diagnostics {
    Diagnostics {
        items: output
            .lines()
            .filter_map(|line| parse_common_line(line, root))
            .collect(),
    }
}

fn parse_common_line(line: &str, root: &Path) -> Option<Diagnostic> {
    let mut fields = line.splitn(4, ':');
    let file = fields.next()?.trim();
    let line = fields.next()?.trim().parse::<usize>().ok()?;
    let col = fields.next()?.trim().parse::<usize>().ok()?;
    let message = fields.next()?.trim();
    if file.is_empty() || line == 0 || col == 0 || message.is_empty() {
        return None;
    }
    let file = PathBuf::from(file);
    Some(Diagnostic {
        file: if file.is_absolute() {
            file
        } else {
            root.join(file)
        },
        line,
        col,
        message: message.to_string(),
        severity: severity_for(message),
    })
}

fn severity_for(message: &str) -> Severity {
    let message = message.to_ascii_lowercase();
    if message.starts_with("warning") {
        Severity::Warning
    } else if message.starts_with("info") || message.starts_with("note") {
        Severity::Info
    } else {
        Severity::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_relative_and_absolute_diagnostics() {
        let root = PathBuf::from("/work/project");
        let output = concat!(
            "src/main.rs:12:7: error: broken thing\n",
            "/tmp/other.py:3:2: warning: suspicious thing\n",
        );

        let parsed = parse_common_output(output, &root);

        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].file, root.join("src/main.rs"));
        assert_eq!((parsed.items[0].line, parsed.items[0].col), (12, 7));
        assert_eq!(parsed.items[0].severity, Severity::Error);
        assert_eq!(parsed.items[1].file, PathBuf::from("/tmp/other.py"));
        assert_eq!(parsed.items[1].severity, Severity::Warning);
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

        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].severity, Severity::Info);
        assert_eq!(parsed.items[0].message, "info: useful note");
    }
}
