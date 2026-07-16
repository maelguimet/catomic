//! Purpose: this file must lazily load safe OpenAI-compatible endpoint settings.
//! Owns: `[llm]` parsing, loopback defaults, validation, and config-path reads.
//! Must not: read API keys, construct HTTP clients, contact endpoints, or load at startup.
//! Invariants: URLs are explicit HTTP(S); timeouts are bounded; missing config is local-first.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io;
use std::path::Path;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080/v1";
const DEFAULT_MODEL: &str = "local-model";
const DEFAULT_KEY_ENV: &str = "OPENAI_API_KEY";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LlmSettings {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) api_key_env: String,
    pub(crate) timeout: Duration,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            api_key_env: DEFAULT_KEY_ENV.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }
}

pub(crate) fn parse(text: &str) -> io::Result<LlmSettings> {
    let mut settings = LlmSettings::default();
    let mut section = "";
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            section = name.trim();
            continue;
        }
        if section != "llm" {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(invalid(index, "LLM setting must use key = value"));
        };
        apply_setting(&mut settings, index, key.trim(), value.trim())?;
    }
    validate(&mut settings)?;
    Ok(settings)
}

pub(crate) fn load_from(path: &Path) -> io::Result<LlmSettings> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LlmSettings::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<LlmSettings> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    match super::big_files::config_path(xdg.as_deref(), home.as_deref()) {
        Some(path) => load_from(&path),
        None => Ok(LlmSettings::default()),
    }
}

fn apply_setting(
    settings: &mut LlmSettings,
    line: usize,
    key: &str,
    value: &str,
) -> io::Result<()> {
    match key {
        "base_url" => settings.base_url = quoted(value, line)?.to_string(),
        "model" => settings.model = quoted(value, line)?.to_string(),
        "api_key_env" => settings.api_key_env = quoted(value, line)?.to_string(),
        "timeout_secs" => {
            let seconds = value
                .parse::<u64>()
                .map_err(|_| invalid(line, "llm.timeout_secs must be an integer"))?;
            settings.timeout = Duration::from_secs(seconds);
        }
        _ => {}
    }
    Ok(())
}

fn validate(settings: &mut LlmSettings) -> io::Result<()> {
    settings.base_url = canonical_base_url(&settings.base_url)?;
    settings.model = settings.model.trim().to_string();
    if settings.model.is_empty() {
        return Err(invalid(0, "llm.model must not be empty"));
    }
    if !valid_env_name(&settings.api_key_env) {
        return Err(invalid(
            0,
            "llm.api_key_env must name an environment variable",
        ));
    }
    if !(1..=600).contains(&settings.timeout.as_secs()) {
        return Err(invalid(0, "llm.timeout_secs must be between 1 and 600"));
    }
    Ok(())
}

fn canonical_base_url(raw: &str) -> io::Result<String> {
    let url = reqwest::Url::parse(raw).map_err(|_| invalid_base_url())?;
    if raw.chars().any(char::is_whitespace)
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_base_url());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn invalid_base_url() -> io::Error {
    invalid(
        0,
        "llm.base_url must be a plain HTTP(S) base URL without credentials, query, or fragment",
    )
}

fn quoted<'a>(value: &'a str, line: usize) -> io::Result<&'a str> {
    let quote = value.as_bytes().first().copied();
    if !matches!(quote, Some(b'\'' | b'"')) || value.as_bytes().last().copied() != quote {
        return Err(invalid(line, "LLM string settings must be quoted"));
    }
    value
        .get(1..value.len().saturating_sub(1))
        .ok_or_else(|| invalid(line, "invalid quoted LLM setting"))
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn invalid(line: usize, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("config line {}: {message}", line + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_configuration_defaults_to_a_loopback_endpoint() {
        let settings = LlmSettings::default();
        assert_eq!(settings.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(settings.model, "local-model");
        assert_eq!(settings.api_key_env, "OPENAI_API_KEY");
    }

    #[test]
    fn parses_llm_settings_and_ignores_other_sections() {
        let settings = parse(
            "[other]\nmodel = \"ignored\"\n[llm]\nbase_url = \"HTTPS://Models.Example:443/v1/\"\n\
             model = \"cat-coder\"\napi_key_env = \"CATOMIC_TOKEN\"\ntimeout_secs = 30\n",
        )
        .unwrap();
        assert_eq!(settings.base_url, "https://models.example/v1");
        assert_eq!(settings.model, "cat-coder");
        assert_eq!(settings.api_key_env, "CATOMIC_TOKEN");
        assert_eq!(settings.timeout, Duration::from_secs(30));
    }

    #[test]
    fn rejects_credentials_bad_env_names_and_unbounded_timeouts() {
        for text in [
            "[llm]\nbase_url = \"https://key@example.test/v1\"\n",
            "[llm]\nbase_url = \"http://\"\n",
            "[llm]\napi_key_env = \"bad-name\"\n",
            "[llm]\ntimeout_secs = 0\n",
            "[llm]\ntimeout_secs = 601\n",
        ] {
            assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn rejects_ambiguous_or_non_http_endpoint_urls() {
        for base_url in [
            "ftp://models.example/v1",
            "https://models.example/v1?tenant=cat",
            "https://models.example/v1#section",
            "https://models.example\t.evil/v1",
            "https://key@example.test/v1",
        ] {
            let text = format!("[llm]\nbase_url = \"{base_url}\"\n");
            assert_eq!(parse(&text).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
