//! Purpose: this file must decode the legacy and named-backend TOML shapes.
//! Owns: raw serde types, compatibility translation, uniqueness, and default selection.
//! Must not: load files, read environment values, resolve executables, spawn, or network.
//! Invariants: mixed legacy/preset shapes fail; the resulting catalog has a valid default.

use std::collections::{BTreeMap, HashSet};
use std::io;

use serde::Deserialize;

use super::{
    inline, validation, BackendAdapter, BackendPreset, CommandInputFormat, CommandOutputFormat,
    HttpBackend, InlineSettings, LlmCatalog, DEFAULT_BASE_URL, DEFAULT_KEY_ENV, DEFAULT_MODEL,
    DEFAULT_PRESET, DEFAULT_TIMEOUT_SECS,
};

const MAX_PRESETS: usize = 128;

pub(super) fn parse(text: &str) -> io::Result<LlmCatalog> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        llm: RawLlm,
        #[serde(default)]
        languages: BTreeMap<String, inline::RawLanguageSettings>,
    }

    let raw_file = toml::from_str::<ConfigFile>(text).map_err(|error| {
        let location = error
            .span()
            .map_or_else(String::new, |span| format!(" near byte {}", span.start));
        invalid(format!(
            "invalid configuration TOML{location}; source text suppressed"
        ))
    })?;
    let (inline, language_inline) = inline::resolve(&raw_file.llm.inline, raw_file.languages)?;
    let raw = raw_file.llm;
    if raw.backends.is_empty() {
        return legacy_catalog(raw, inline, language_inline);
    }
    if raw.has_legacy_fields() {
        return Err(invalid(
            "llm legacy endpoint fields cannot be combined with llm.backends",
        ));
    }
    preset_catalog(raw, inline, language_inline)
}

#[derive(Default, Deserialize)]
struct RawLlm {
    default: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    timeout_secs: Option<u64>,
    #[serde(default)]
    inline: inline::RawInlineSettings,
    #[serde(default)]
    backends: Vec<RawBackend>,
}

impl RawLlm {
    fn has_legacy_fields(&self) -> bool {
        self.base_url.is_some()
            || self.model.is_some()
            || self.api_key_env.is_some()
            || self.timeout_secs.is_some()
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawBackend {
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible {
        name: String,
        model: String,
        base_url: String,
        api_key_env: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        header_envs: BTreeMap<String, String>,
        #[serde(default)]
        models: Vec<String>,
        #[serde(default)]
        discovery: bool,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "enabled_by_default")]
        enabled: bool,
    },
    #[serde(rename = "command")]
    Command {
        name: String,
        model: String,
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default = "default_command_input")]
        input: CommandInputFormat,
        output: CommandOutputFormat,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "enabled_by_default")]
        enabled: bool,
    },
}

fn legacy_catalog(
    raw: RawLlm,
    inline: InlineSettings,
    language_inline: BTreeMap<String, inline::RawInlineSettings>,
) -> io::Result<LlmCatalog> {
    if raw
        .default
        .as_deref()
        .is_some_and(|name| name != DEFAULT_PRESET)
    {
        return Err(invalid(
            "llm.default does not name the implicit local preset",
        ));
    }
    let api_key_env = raw
        .api_key_env
        .unwrap_or_else(|| DEFAULT_KEY_ENV.to_string());
    validation::validate_env_name(&api_key_env, "llm.api_key_env")?;
    let preset = BackendPreset {
        name: DEFAULT_PRESET.to_string(),
        model: validation::validated_model(raw.model.unwrap_or_else(|| DEFAULT_MODEL.to_string()))?,
        enabled: true,
        adapter: BackendAdapter::OpenAiCompatible(HttpBackend {
            base_url: validation::canonical_base_url(
                &raw.base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            )?,
            api_key_env: Some(api_key_env),
            credential_required: false,
            headers: BTreeMap::new(),
            header_envs: BTreeMap::new(),
            models: Vec::new(),
            discovery: false,
            timeout: validation::bounded_timeout(
                raw.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
                "llm.timeout_secs",
            )?,
        }),
    };
    Ok(LlmCatalog {
        default: DEFAULT_PRESET.to_string(),
        presets: vec![preset],
        inline,
        language_inline,
    })
}

fn preset_catalog(
    raw: RawLlm,
    inline: InlineSettings,
    language_inline: BTreeMap<String, inline::RawInlineSettings>,
) -> io::Result<LlmCatalog> {
    if raw.backends.len() > MAX_PRESETS {
        return Err(invalid("llm.backends exceeds 128 presets"));
    }
    let mut names = HashSet::new();
    let mut presets = Vec::with_capacity(raw.backends.len());
    for backend in raw.backends {
        let preset = validate_backend(backend)?;
        if !names.insert(preset.name.clone()) {
            return Err(invalid(format!("duplicate llm backend {:?}", preset.name)));
        }
        presets.push(preset);
    }
    let default = raw.default.unwrap_or_else(|| presets[0].name.clone());
    if !names.contains(&default) {
        return Err(invalid(format!(
            "llm.default names unknown backend {default:?}"
        )));
    }
    Ok(LlmCatalog {
        default,
        presets,
        inline,
        language_inline,
    })
}

fn validate_backend(raw: RawBackend) -> io::Result<BackendPreset> {
    match raw {
        RawBackend::OpenAiCompatible {
            name,
            model,
            base_url,
            api_key_env,
            headers,
            header_envs,
            models,
            discovery,
            timeout_secs,
            enabled,
        } => validation::http_backend(
            name,
            model,
            base_url,
            api_key_env,
            headers,
            header_envs,
            models,
            discovery,
            timeout_secs,
            enabled,
        ),
        RawBackend::Command {
            name,
            model,
            program,
            args,
            input,
            output,
            timeout_secs,
            enabled,
        } => validation::command_backend(
            name,
            model,
            program,
            args,
            input,
            output,
            timeout_secs,
            enabled,
        ),
    }
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

fn default_command_input() -> CommandInputFormat {
    CommandInputFormat::StdinTextV1
}

fn enabled_by_default() -> bool {
    true
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
