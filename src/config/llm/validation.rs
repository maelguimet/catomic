//! Purpose: this file must turn raw preset fields into bounded adapter settings.
//! Owns: URL, name, model, env, header, argv, and timeout validation.
//! Must not: decode/load config, inspect environment values, resolve programs, spawn, or network.
//! Invariants: validated strings are control-free and all variable-size collections are capped.
//! Phase: post-v0.1 model/backend configuration validation.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;
use std::time::Duration;

use super::{
    BackendAdapter, BackendPreset, CommandBackend, CommandInputFormat, CommandOutputFormat,
    HttpBackend,
};

const MAX_MODELS: usize = 128;

#[allow(clippy::too_many_arguments)]
pub(super) fn http_backend(
    name: String,
    model: String,
    base_url: String,
    api_key_env: Option<String>,
    headers: BTreeMap<String, String>,
    header_envs: BTreeMap<String, String>,
    models: Vec<String>,
    discovery: bool,
    timeout_secs: u64,
    enabled: bool,
) -> io::Result<BackendPreset> {
    if let Some(name) = api_key_env.as_deref() {
        validate_env_name(name, "api_key_env")?;
    }
    let credential_required = api_key_env.is_some();
    validate_headers(&headers, &header_envs)?;
    if credential_required
        && headers
            .keys()
            .chain(header_envs.keys())
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    {
        return Err(invalid(
            "api_key_env cannot be combined with an explicit Authorization header",
        ));
    }
    let model = validated_model(model)?;
    let models = validated_models(models, &model)?;
    Ok(BackendPreset {
        name: validated_name(name)?,
        model,
        enabled,
        adapter: BackendAdapter::OpenAiCompatible(HttpBackend {
            base_url: canonical_base_url(&base_url)?,
            api_key_env,
            credential_required,
            headers,
            header_envs,
            models,
            discovery,
            timeout: bounded_timeout(timeout_secs, "llm.backends[].timeout_secs")?,
        }),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn command_backend(
    name: String,
    model: String,
    program: String,
    args: Vec<String>,
    input: CommandInputFormat,
    output: CommandOutputFormat,
    timeout_secs: u64,
    enabled: bool,
) -> io::Result<BackendPreset> {
    validate_program(&program)?;
    if args.len() > 64
        || args
            .iter()
            .any(|arg| arg.len() > 8_192 || arg.contains('\0'))
    {
        return Err(invalid(
            "command backend args exceed safe bounds or contain NUL",
        ));
    }
    Ok(BackendPreset {
        name: validated_name(name)?,
        model: validated_model(model)?,
        enabled,
        adapter: BackendAdapter::Command(CommandBackend {
            program,
            args,
            input,
            output,
            timeout: bounded_timeout(timeout_secs, "llm.backends[].timeout_secs")?,
        }),
    })
}

fn validated_name(raw: String) -> io::Result<String> {
    let name = raw.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(invalid(
            "llm backend names must be 1-64 printable characters",
        ));
    }
    Ok(name)
}

pub(super) fn validated_model(raw: String) -> io::Result<String> {
    let model = raw.trim().to_string();
    if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        return Err(invalid(
            "LLM model identifiers must be 1-256 printable bytes",
        ));
    }
    Ok(model)
}

fn validated_models(models: Vec<String>, primary: &str) -> io::Result<Vec<String>> {
    if models.len() > MAX_MODELS {
        return Err(invalid("llm backend models exceeds 128 entries"));
    }
    let mut seen = HashSet::new();
    let mut valid = Vec::new();
    for model in models {
        let model = validated_model(model)?;
        if model != primary && seen.insert(model.clone()) {
            valid.push(model);
        }
    }
    Ok(valid)
}

fn validate_program(program: &str) -> io::Result<()> {
    let path = Path::new(program);
    let bare = path.components().count() == 1;
    if program.is_empty()
        || program.contains('\0')
        || program.chars().any(char::is_control)
        || (!path.is_absolute() && !bare)
    {
        return Err(invalid(
            "command backend program must be an absolute path or a bare executable name",
        ));
    }
    Ok(())
}

fn validate_headers(
    headers: &BTreeMap<String, String>,
    header_envs: &BTreeMap<String, String>,
) -> io::Result<()> {
    if headers.len().saturating_add(header_envs.len()) > 32 {
        return Err(invalid("LLM HTTP backends support at most 32 headers"));
    }
    let mut seen = HashSet::new();
    for (name, value) in headers {
        validate_header_name(name)?;
        if credential_header_name(name) {
            return Err(invalid(format!(
                "credential-like HTTP header {name:?} must use header_envs"
            )));
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(invalid("HTTP header names must be unique ignoring case"));
        }
        reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| invalid(format!("invalid static HTTP header {name:?}")))?;
    }
    for (name, env) in header_envs {
        validate_header_name(name)?;
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(invalid("HTTP header names must be unique ignoring case"));
        }
        validate_env_name(env, "header_envs value")?;
    }
    Ok(())
}

fn credential_header_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
    ) || name.contains("token")
        || name.contains("api-key")
        || name.contains("apikey")
        || name.ends_with("-key")
}

fn validate_header_name(name: &str) -> io::Result<()> {
    reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map(|_| ())
        .map_err(|_| invalid(format!("invalid HTTP header name {name:?}")))
}

pub(super) fn validate_env_name(name: &str, field: &str) -> io::Result<()> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "{field} must name an environment variable"
        )))
    }
}

pub(super) fn bounded_timeout(seconds: u64, field: &str) -> io::Result<Duration> {
    if !(1..=600).contains(&seconds) {
        return Err(invalid(format!("{field} must be between 1 and 600")));
    }
    Ok(Duration::from_secs(seconds))
}

pub(super) fn canonical_base_url(raw: &str) -> io::Result<String> {
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
    invalid("LLM base_url must be plain HTTP(S) without credentials, query, or fragment")
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
