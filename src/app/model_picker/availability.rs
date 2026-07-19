//! Purpose: this file must classify preset availability without invoking a backend.
//! Owns: endpoint locality, credential-name presence checks, and executable identity lookup.
//! Must not: read secret bytes, run version probes, construct clients, spawn, or network.
//! Invariants: picker checks are cheap and context-free; every displayed string is control-free.
//! Phase: post-v0.1 model/backend picker.

use std::ffi::CString;
use std::net::IpAddr;

use crate::config::llm::{BackendAdapter, BackendPreset, CommandOutputFormat};
use crate::llm::backend::BackendErrorKind;

pub(super) struct Snapshot {
    pub(super) destination: String,
    pub(super) command_available: Option<bool>,
}

pub(super) fn summary(
    preset: &BackendPreset,
    discovered: bool,
    health: Option<BackendErrorKind>,
    command_available: Option<bool>,
) -> String {
    if !preset.enabled {
        return "disabled".to_string();
    }
    match &preset.adapter {
        BackendAdapter::OpenAiCompatible(http) => {
            let missing = http
                .api_key_env
                .as_deref()
                .filter(|_| http.credential_required)
                .into_iter()
                .chain(http.header_envs.values().map(String::as_str))
                .find(|name| !environment_name_present(name));
            if let Some(name) = missing {
                format!("missing credential {name}")
            } else if let Some(summary) = failed_summary(&preset.adapter, health) {
                summary.to_string()
            } else if discovered {
                "ready (cached discovery)".to_string()
            } else {
                "endpoint unchecked".to_string()
            }
        }
        BackendAdapter::Command(command) => {
            if command_available != Some(true) {
                "missing executable".to_string()
            } else if let Some(summary) = failed_summary(&preset.adapter, health) {
                summary.to_string()
            } else {
                format!("ready; {} unchecked", output_label(command.output))
            }
        }
    }
}

pub(super) fn inspect(preset: &BackendPreset) -> Snapshot {
    match &preset.adapter {
        BackendAdapter::OpenAiCompatible(http) => Snapshot {
            destination: format!("{} {}", endpoint_location(&http.base_url), http.base_url),
            command_available: None,
        },
        BackendAdapter::Command(command) => match crate::llm::executable::resolve(&command.program)
        {
            Ok(path) => Snapshot {
                destination: format!("local {}", crate::llm::executable::safe_identity(&path)),
                command_available: Some(true),
            },
            Err(_) => Snapshot {
                destination: format!("local {}", safe_text(&command.program)),
                command_available: Some(false),
            },
        },
    }
}

fn failed_summary(
    adapter: &BackendAdapter,
    health: Option<BackendErrorKind>,
) -> Option<&'static str> {
    match (adapter, health) {
        (BackendAdapter::Command(_), Some(BackendErrorKind::Incompatible)) => {
            Some("incompatible CLI output/version")
        }
        (BackendAdapter::OpenAiCompatible(_), Some(BackendErrorKind::Unreachable)) => {
            Some("endpoint unreachable (last attempt)")
        }
        (_, Some(BackendErrorKind::Incompatible)) => Some("incompatible backend output/version"),
        (_, Some(BackendErrorKind::Unreachable)) => Some("backend unreachable (last attempt)"),
        (_, Some(BackendErrorKind::TimedOut)) => Some("timed out (last attempt)"),
        (_, Some(BackendErrorKind::OutputTooLarge)) => Some("oversized output (last attempt)"),
        (_, Some(BackendErrorKind::Unavailable)) => Some("unavailable (last attempt)"),
        (_, Some(BackendErrorKind::Failed)) => Some("failed (last attempt)"),
        (_, Some(BackendErrorKind::Cancelled) | None) => None,
    }
}

fn endpoint_location(base_url: &str) -> &'static str {
    let local = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| is_loopback_host(&host));
    if local {
        "local"
    } else {
        "remote"
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn environment_name_present(name: &str) -> bool {
    let Ok(name) = CString::new(name) else {
        return false;
    };
    // SAFETY: getenv returns an environment-owned pointer. The picker checks only nullness and
    // deliberately never dereferences the pointer, so no credential bytes are read or copied.
    unsafe { !libc::getenv(name.as_ptr()).is_null() }
}

fn output_label(output: CommandOutputFormat) -> &'static str {
    match output {
        CommandOutputFormat::ClaudeJsonV1 => "claude-json-v1",
        CommandOutputFormat::CodexJsonlV1 => "codex-jsonl-v1",
    }
}

fn safe_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { '�' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_local_remote_disabled_and_missing_command_without_probes() {
        let catalog = crate::config::llm::parse(
            "[[llm.backends]]\nname='local'\ntype='openai-compatible'\nbase_url='http://127.0.0.1:8/v1'\nmodel='a'\n[[llm.backends]]\nname='remote'\ntype='openai-compatible'\nbase_url='https://models.example/v1'\nmodel='b'\n[[llm.backends]]\nname='off'\ntype='command'\nprogram='missing-catomic-test'\nmodel='c'\noutput='claude-json-v1'\nenabled=false\n",
        )
        .unwrap();
        assert!(inspect(catalog.find("local").unwrap())
            .destination
            .starts_with("local "));
        assert!(inspect(catalog.find("remote").unwrap())
            .destination
            .starts_with("remote "));
        assert_eq!(
            summary(catalog.find("off").unwrap(), false, None, Some(false)),
            "disabled"
        );
    }
}
