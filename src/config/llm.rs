//! Purpose: this file must lazily load and validate named LLM backend presets.
//! Owns: `[llm]` compatibility translation, preset schemas, and safe display metadata.
//! Must not: read secret values, resolve executables, construct clients, spawn, or network.
//! Invariants: every preset is bounded and uniquely named; legacy config remains local-first.
//! Phase: post-v0.1 model/backend selection.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080/v1";
const DEFAULT_MODEL: &str = "local-model";
const DEFAULT_KEY_ENV: &str = "OPENAI_API_KEY";
const DEFAULT_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PRESET: &str = "local";

mod schema;
mod validation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LlmCatalog {
    pub(crate) default: String,
    pub(crate) presets: Vec<BackendPreset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BackendPreset {
    pub(crate) name: String,
    pub(crate) model: String,
    pub(crate) enabled: bool,
    pub(crate) adapter: BackendAdapter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BackendAdapter {
    OpenAiCompatible(HttpBackend),
    Command(CommandBackend),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpBackend {
    pub(crate) base_url: String,
    pub(crate) api_key_env: Option<String>,
    pub(crate) credential_required: bool,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) header_envs: BTreeMap<String, String>,
    pub(crate) models: Vec<String>,
    pub(crate) discovery: bool,
    pub(crate) timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandBackend {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) input: CommandInputFormat,
    pub(crate) output: CommandOutputFormat,
    pub(crate) timeout: Duration,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum CommandInputFormat {
    #[serde(rename = "stdin-text-v1")]
    StdinTextV1,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum CommandOutputFormat {
    #[serde(rename = "claude-json-v1")]
    ClaudeJsonV1,
    #[serde(rename = "codex-jsonl-v1")]
    CodexJsonlV1,
}

impl Default for LlmCatalog {
    fn default() -> Self {
        Self {
            default: DEFAULT_PRESET.to_string(),
            presets: vec![BackendPreset {
                name: DEFAULT_PRESET.to_string(),
                model: DEFAULT_MODEL.to_string(),
                enabled: true,
                adapter: BackendAdapter::OpenAiCompatible(HttpBackend {
                    base_url: DEFAULT_BASE_URL.to_string(),
                    api_key_env: Some(DEFAULT_KEY_ENV.to_string()),
                    credential_required: false,
                    headers: BTreeMap::new(),
                    header_envs: BTreeMap::new(),
                    models: Vec::new(),
                    discovery: false,
                    timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                }),
            }],
        }
    }
}

impl LlmCatalog {
    pub(crate) fn default_preset(&self) -> &BackendPreset {
        self.find(&self.default).expect("validated default preset")
    }

    pub(crate) fn find(&self, name: &str) -> Option<&BackendPreset> {
        self.presets.iter().find(|preset| preset.name == name)
    }
}

impl BackendPreset {
    pub(crate) fn with_model(&self, model: String) -> Self {
        let mut preset = self.clone();
        preset.model = model;
        preset
    }

    pub(crate) fn adapter_label(&self) -> &'static str {
        match self.adapter {
            BackendAdapter::OpenAiCompatible(_) => "http",
            BackendAdapter::Command(_) => "command",
        }
    }

    pub(crate) fn destination(&self) -> &str {
        match &self.adapter {
            BackendAdapter::OpenAiCompatible(http) => &http.base_url,
            BackendAdapter::Command(command) => &command.program,
        }
    }
}

pub(crate) fn parse(text: &str) -> io::Result<LlmCatalog> {
    schema::parse(text)
}

pub(crate) fn load_from(path: &Path) -> io::Result<LlmCatalog> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LlmCatalog::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<LlmCatalog> {
    match super::user_file::optional_path() {
        Some(path) => load_from(&path),
        None => Ok(LlmCatalog::default()),
    }
}

pub(crate) fn validated_model(raw: String) -> io::Result<String> {
    validation::validated_model(raw)
}

#[cfg(test)]
mod tests;
