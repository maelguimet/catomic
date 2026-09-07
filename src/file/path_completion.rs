//! Explicit filename completion. Reads one directory only when requested; a
//! truncated enumeration never produces a supposedly unique or common match.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unicode_segmentation::UnicodeSegmentation;

const MAX_ENTRIES: usize = 4096;
const SCAN_BUDGET: Duration = Duration::from_millis(25);

#[cfg(test)]
thread_local! {
    pub(crate) static REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) struct Completion {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) text: String,
    pub(crate) notice: Option<String>,
}

pub(crate) fn complete(
    text: &str,
    caret: usize,
    home: Option<&OsStr>,
) -> Result<Completion, String> {
    #[cfg(test)]
    REQUESTS.set(REQUESTS.get() + 1);
    complete_with_limit(text, caret, home, MAX_ENTRIES)
}

fn complete_with_limit(
    text: &str,
    caret: usize,
    home: Option<&OsStr>,
    max_entries: usize,
) -> Result<Completion, String> {
    let before = &text[..caret];
    if before == "~" {
        if home.is_none() {
            return Err("Cannot complete ~: HOME is not set.".into());
        }
        return Ok(Completion {
            start: 0,
            end: caret,
            text: "~/".into(),
            notice: None,
        });
    }
    let start = before.rfind('/').map_or(0, |byte| byte + 1);
    let end = text[caret..]
        .find('/')
        .map_or(text.len(), |byte| caret + byte);
    let prefix = &text[start..caret];
    let directory = &text[..start];
    let directory = if let Some(tail) = directory.strip_prefix("~/") {
        PathBuf::from(home.ok_or("Cannot complete ~: HOME is not set.")?).join(tail)
    } else if directory.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(directory)
    };
    let began = Instant::now();
    let entries = std::fs::read_dir(&directory).map_err(|e| format!("Completion error: {e}"))?;
    let mut count = 0;
    let mut common = String::new();
    let mut unique_path = None;
    for (index, entry) in entries.enumerate() {
        if index >= max_entries || began.elapsed() >= SCAN_BUDGET {
            return Err("Completion scan limit reached; type a more specific directory.".into());
        }
        let entry = entry.map_err(|e| format!("Completion error: {e}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
            continue;
        }
        count += 1;
        if count == 1 {
            common = name.to_owned();
            unique_path = Some(entry.path());
        } else {
            unique_path = None;
            let bytes = common
                .bytes()
                .zip(name.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            // Only publish a shared extended-grapheme boundary of both names.
            let boundary = common
                .grapheme_indices(true)
                .map(|(byte, _)| byte)
                .chain(std::iter::once(common.len()))
                .rfind(|&byte| {
                    byte <= bytes
                        && (byte == name.len()
                            || name.grapheme_indices(true).any(|(b, _)| b == byte))
                })
                .unwrap_or(0);
            common.truncate(boundary);
        }
    }
    if count == 0 {
        return Err("No matching paths.".into());
    }
    if common.len() < prefix.len() {
        return Err(format!(
            "{count} matching paths; type more to disambiguate."
        ));
    }
    if unique_path.as_deref().is_some_and(Path::is_dir) && !text[end..].starts_with('/') {
        common.push('/');
    }
    Ok(Completion {
        start,
        end,
        text: common,
        notice: (count > 1).then(|| format!("{count} matching paths; type more to disambiguate.")),
    })
}

#[cfg(test)]
mod tests;
