//! Purpose: guard ambiguous multi-file arguments before terminal setup.
//! Owns: read-only path classification and the actionable startup diagnostic.
//! Must not: create, open, overwrite, or otherwise mutate a path.
//! Invariants: one missing path among several requires an explicit opt-in.
//! Phase: beta CLI safety hardening.

use std::fmt::Write as _;
use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathStatus {
    Existing,
    Missing,
    Unavailable(io::ErrorKind),
}

struct InspectedPath<'a> {
    argument: &'a str,
    status: PathStatus,
}

pub(crate) fn check(files: &[String], allow_missing: bool) -> Result<(), String> {
    if files.len() < 2 || allow_missing {
        return Ok(());
    }

    let paths: Vec<_> = files
        .iter()
        .map(|argument| InspectedPath {
            argument,
            status: inspect_path(argument),
        })
        .collect();
    if paths.iter().all(|path| path.status != PathStatus::Missing) {
        return Ok(());
    }

    Err(format_diagnostic(&paths))
}

fn inspect_path(argument: &str) -> PathStatus {
    match Path::new(argument).try_exists() {
        Ok(true) => PathStatus::Existing,
        Ok(false) => PathStatus::Missing,
        Err(error) => PathStatus::Unavailable(error.kind()),
    }
}

fn format_diagnostic(paths: &[InspectedPath<'_>]) -> String {
    let mut diagnostic = format!(
        "catomic: ambiguous multi-file arguments; refusing to start\n\n\
         Catomic parsed {} file arguments:\n",
        paths.len()
    );
    for (index, path) in paths.iter().enumerate() {
        let _ = writeln!(
            diagnostic,
            "  {}. [{}] {:?}",
            index + 1,
            status_label(path.status),
            path.argument
        );
    }

    append_quoting_hint(&mut diagnostic, paths);
    append_explicit_opt_in(&mut diagnostic, paths);
    diagnostic.push_str(
        "\nNo files were opened, created, or changed.\n\
         Switch buffers with Alt+PageUp / Alt+PageDown.\n",
    );
    diagnostic
}

fn status_label(status: PathStatus) -> String {
    match status {
        PathStatus::Existing => "existing".to_string(),
        PathStatus::Missing => "missing".to_string(),
        PathStatus::Unavailable(kind) => format!("status unavailable: {kind:?}"),
    }
}

fn append_quoting_hint(diagnostic: &mut String, paths: &[InspectedPath<'_>]) {
    let joined = paths
        .iter()
        .map(|path| path.argument)
        .collect::<Vec<_>>()
        .join(" ");
    let exact_match = Path::new(&joined).try_exists().unwrap_or(false);
    let explanation = if exact_match {
        "An existing path matches these arguments joined with spaces.\n\
         If you meant that one file, quote it:"
    } else {
        "If you meant one filename containing spaces, quote it, for example:"
    };
    let _ = write!(diagnostic, "\n{explanation}\n");
    if let Some(command) = format_command("catomic", &[joined.as_str()], true) {
        let _ = writeln!(diagnostic, "  {command}");
    } else {
        diagnostic.push_str("  (quote the intended single filename in your shell)\n");
    }
}

fn append_explicit_opt_in(diagnostic: &mut String, paths: &[InspectedPath<'_>]) {
    diagnostic.push_str(
        "\nTo intentionally open/create all parsed buffers, rerun with --allow-missing:\n",
    );
    let arguments: Vec<_> = paths.iter().map(|path| path.argument).collect();
    if let Some(command) = format_command("catomic --allow-missing", &arguments, true) {
        let _ = writeln!(diagnostic, "  {command}");
    } else {
        diagnostic.push_str("  add --allow-missing before the same file arguments\n");
    }
}

fn format_command(program: &str, arguments: &[&str], literal_options: bool) -> Option<String> {
    let quoted: Option<Vec<_>> = arguments
        .iter()
        .map(|argument| shell_quote(argument))
        .collect();
    let quoted = quoted?;
    let separator = literal_options && arguments.iter().any(|argument| argument.starts_with('-'));
    let mut command = program.to_string();
    if separator {
        command.push_str(" --");
    }
    for argument in quoted {
        command.push(' ');
        command.push_str(&argument);
    }
    Some(command)
}

fn shell_quote(argument: &str) -> Option<String> {
    if argument.chars().any(is_unsafe_to_echo) {
        return None;
    }
    Some(format!("'{}'", argument.replace('\'', "'\\''")))
}

fn is_unsafe_to_echo(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests;
