//! Purpose: this file must validate bounded inline-clanker syntax and per-extension overrides.
//! Owns: inline defaults, marker disambiguation, limits, and extension lookup.
//! Must not: load files, inspect buffers, read secrets, construct clients, or perform network I/O.
//! Invariants: control markers are bounded and unambiguous; queue and warning limits are finite.
//! Phase: issue #65 one-key inline clanker workflow.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::Deserialize;

const DEFAULT_INSTRUCTION_PREFIX: &str = ">>";
const DEFAULT_CONTEXT_OPEN: &str = "<catblock>";
const DEFAULT_CONTEXT_CLOSE: &str = "</catblock>";
const DEFAULT_WARN_LINES: usize = 500;
const DEFAULT_QUEUE_LIMIT: usize = 16;
const MAX_MARKER_BYTES: usize = 64;
const MAX_WARN_LINES: usize = 2_000;
const MAX_QUEUE_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineBlockMode {
    Combined,
    Queued,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineSettings {
    pub(crate) instruction_prefix: String,
    pub(crate) instruction_suffix: String,
    pub(crate) context_open: String,
    pub(crate) context_close: String,
    pub(crate) warn_lines: usize,
    pub(crate) block_mode: InlineBlockMode,
    pub(crate) queue_limit: usize,
    pub(crate) stop_on_error: bool,
    pub(crate) remove_instruction_after_apply: bool,
}

impl Default for InlineSettings {
    fn default() -> Self {
        Self {
            instruction_prefix: DEFAULT_INSTRUCTION_PREFIX.to_string(),
            instruction_suffix: String::new(),
            context_open: DEFAULT_CONTEXT_OPEN.to_string(),
            context_close: DEFAULT_CONTEXT_CLOSE.to_string(),
            warn_lines: DEFAULT_WARN_LINES,
            block_mode: InlineBlockMode::Combined,
            queue_limit: DEFAULT_QUEUE_LIMIT,
            stop_on_error: true,
            remove_instruction_after_apply: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(super) struct RawInlineSettings {
    instruction_prefix: Option<String>,
    instruction_suffix: Option<String>,
    context_open: Option<String>,
    context_close: Option<String>,
    warn_lines: Option<usize>,
    block_mode: Option<String>,
    queue_limit: Option<usize>,
    stop_on_error: Option<bool>,
    remove_instruction_after_apply: Option<bool>,
}

#[derive(Default, Deserialize)]
pub(super) struct RawLanguageSettings {
    #[serde(default)]
    llm: RawLanguageLlmSettings,
}

#[derive(Default, Deserialize)]
struct RawLanguageLlmSettings {
    #[serde(default)]
    inline: RawInlineSettings,
}

pub(super) fn resolve(
    raw: &RawInlineSettings,
    languages: BTreeMap<String, RawLanguageSettings>,
) -> io::Result<(InlineSettings, BTreeMap<String, RawInlineSettings>)> {
    let mut settings = InlineSettings::default();
    apply(&mut settings, raw)?;
    validate(&settings)?;
    let mut overrides = BTreeMap::new();
    for (raw_extension, language) in languages {
        let extension = normalize_extension(&raw_extension);
        if extension.is_empty() || extension.chars().any(char::is_whitespace) {
            return Err(invalid("language extension must not be empty"));
        }
        if overrides
            .insert(extension.clone(), language.llm.inline)
            .is_some()
        {
            return Err(invalid(format!(
                "language extension {raw_extension:?} duplicates {extension:?}"
            )));
        }
    }
    for (extension, raw_override) in &overrides {
        let mut effective = settings.clone();
        apply(&mut effective, raw_override)?;
        validate(&effective).map_err(|error| {
            invalid(format!(
                "invalid inline settings for extension {extension:?}: {error}"
            ))
        })?;
    }
    Ok((settings, overrides))
}

pub(super) fn for_path(
    defaults: &InlineSettings,
    overrides: &BTreeMap<String, RawInlineSettings>,
    path: Option<&Path>,
) -> io::Result<InlineSettings> {
    let mut settings = defaults.clone();
    if let Some(extension) = extension_for_path(path) {
        if let Some(raw) = overrides.get(&extension) {
            apply(&mut settings, raw)?;
        }
    }
    validate(&settings)?;
    Ok(settings)
}

fn apply(settings: &mut InlineSettings, raw: &RawInlineSettings) -> io::Result<()> {
    if let Some(value) = &raw.instruction_prefix {
        settings.instruction_prefix.clone_from(value);
    }
    if let Some(value) = &raw.instruction_suffix {
        settings.instruction_suffix.clone_from(value);
    }
    if let Some(value) = &raw.context_open {
        settings.context_open.clone_from(value);
    }
    if let Some(value) = &raw.context_close {
        settings.context_close.clone_from(value);
    }
    if let Some(value) = raw.warn_lines {
        settings.warn_lines = value;
    }
    if let Some(value) = raw.queue_limit {
        settings.queue_limit = value;
    }
    if let Some(value) = raw.stop_on_error {
        settings.stop_on_error = value;
    }
    if let Some(value) = raw.remove_instruction_after_apply {
        settings.remove_instruction_after_apply = value;
    }
    if let Some(value) = raw.block_mode.as_deref() {
        settings.block_mode = match value.to_ascii_lowercase().as_str() {
            "queued" => InlineBlockMode::Queued,
            "combined" => InlineBlockMode::Combined,
            _ => return Err(invalid("llm.inline.block_mode must be combined or queued")),
        };
    }
    Ok(())
}

fn validate(settings: &InlineSettings) -> io::Result<()> {
    for (name, marker) in [
        ("instruction_prefix", settings.instruction_prefix.as_str()),
        ("context_open", settings.context_open.as_str()),
        ("context_close", settings.context_close.as_str()),
    ] {
        validate_marker(name, marker, false)?;
    }
    validate_marker("instruction_suffix", &settings.instruction_suffix, true)?;
    let mut markers = vec![
        settings.instruction_prefix.as_str(),
        settings.context_open.as_str(),
        settings.context_close.as_str(),
    ];
    if !settings.instruction_suffix.is_empty() {
        markers.push(settings.instruction_suffix.as_str());
    }
    for (index, left) in markers.iter().enumerate() {
        for right in &markers[index + 1..] {
            if left.starts_with(right) || right.starts_with(left) {
                return Err(invalid(
                    "llm.inline markers must not overlap or be ambiguous",
                ));
            }
        }
    }
    if !(1..=MAX_WARN_LINES).contains(&settings.warn_lines) {
        return Err(invalid("llm.inline.warn_lines must be between 1 and 2000"));
    }
    if !(1..=MAX_QUEUE_LIMIT).contains(&settings.queue_limit) {
        return Err(invalid("llm.inline.queue_limit must be between 1 and 64"));
    }
    Ok(())
}

fn validate_marker(name: &str, marker: &str, empty_allowed: bool) -> io::Result<()> {
    if (!empty_allowed && marker.is_empty())
        || marker.len() > MAX_MARKER_BYTES
        || marker.trim() != marker
        || marker.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "llm.inline.{name} must be {}at most {MAX_MARKER_BYTES} bytes without outer whitespace or controls",
            if empty_allowed { "empty or " } else { "non-empty and " }
        )));
    }
    Ok(())
}

fn extension_for_path(path: Option<&Path>) -> Option<String> {
    path.and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(normalize_extension)
}

fn normalize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
