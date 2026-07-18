//! Purpose: this file must map validated presets into one confirmed generation boundary.
//! Owns: post-confirmation secret/executable resolution, messages, and adapter dispatch.
//! Must not: load config, collect context, persist clients, interpret proposals, or write files.
//! Invariants: resolution occurs only after confirmation; both adapters return plain proposals.
//! Phase: post-v0.1 common LLM backend abstraction.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::llm::{BackendAdapter, BackendPreset};

use super::command_adapter::ResolvedCommand;
use super::openai_compat::{ChatMessage, LlmConfig, LlmError, OpenAiCompatClient};

#[derive(Clone)]
pub(crate) struct ConfirmedBackend {
    adapter: ConfirmedAdapter,
}

#[derive(Clone)]
enum ConfirmedAdapter {
    Http(LlmConfig),
    Command(ResolvedCommand),
}

#[derive(Clone, Debug)]
pub(crate) struct BackendMessage {
    pub(crate) role: MessageRole,
    pub(crate) content: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackendErrorKind {
    Cancelled,
    Unavailable,
    Unreachable,
    Incompatible,
    TimedOut,
    OutputTooLarge,
    Failed,
}

#[derive(Debug)]
pub(crate) struct BackendError {
    pub(crate) kind: BackendErrorKind,
    message: String,
}

pub(crate) struct BackendRunner<'a> {
    adapter: RunnerAdapter,
    cancel: &'a AtomicBool,
}

enum RunnerAdapter {
    Http {
        runtime: tokio::runtime::Runtime,
        client: OpenAiCompatClient,
    },
    Command(ResolvedCommand),
}

impl ConfirmedBackend {
    pub(crate) fn resolve(preset: &BackendPreset) -> Result<Self, BackendError> {
        if !preset.enabled {
            return Err(BackendError::new(
                BackendErrorKind::Unavailable,
                "preset is disabled",
            ));
        }
        let adapter = match &preset.adapter {
            BackendAdapter::OpenAiCompatible(http) => {
                ConfirmedAdapter::Http(resolve_http(http, &preset.model)?)
            }
            BackendAdapter::Command(command) => {
                let program = super::executable::resolve(&command.program).map_err(|_| {
                    BackendError::new(
                        BackendErrorKind::Unavailable,
                        "configured command executable is unavailable",
                    )
                })?;
                ConfirmedAdapter::Command(ResolvedCommand {
                    program,
                    args: command.args.clone(),
                    input: command.input,
                    output: command.output,
                    timeout: command.timeout,
                })
            }
        };
        Ok(Self { adapter })
    }

    pub(crate) fn destination(&self) -> String {
        match &self.adapter {
            ConfirmedAdapter::Http(config) => config.base_url.clone(),
            ConfirmedAdapter::Command(command) => {
                super::executable::safe_identity(&command.program)
            }
        }
    }
}

pub(crate) fn display_destination(preset: &BackendPreset) -> String {
    match &preset.adapter {
        BackendAdapter::OpenAiCompatible(http) => http.base_url.clone(),
        BackendAdapter::Command(command) => super::executable::resolve(&command.program)
            .map(|path| super::executable::safe_identity(&path))
            .unwrap_or_else(|_| format!("{} (unavailable)", command.program)),
    }
}

pub(super) fn resolve_http(
    http: &crate::config::llm::HttpBackend,
    model: &str,
) -> Result<LlmConfig, BackendError> {
    let api_key = match http.api_key_env.as_deref() {
        Some(name) => read_secret(name, http.credential_required)?,
        None => None,
    };
    let mut headers = http
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    for (name, env) in &http.header_envs {
        let value = read_secret(env, true)?.expect("required secret");
        headers.push((name.clone(), value));
    }
    Ok(LlmConfig {
        base_url: http.base_url.clone(),
        api_key,
        headers,
        model: model.to_string(),
        timeout: http.timeout,
    })
}

fn read_secret(name: &str, required: bool) -> Result<Option<String>, BackendError> {
    let value = std::env::var(name).ok().filter(|value| !value.is_empty());
    if required && value.is_none() {
        return Err(BackendError::new(
            BackendErrorKind::Unavailable,
            format!("required credential environment variable {name} is missing"),
        ));
    }
    Ok(value)
}

impl<'a> BackendRunner<'a> {
    pub(crate) fn new(
        backend: ConfirmedBackend,
        cancel: &'a AtomicBool,
    ) -> Result<Self, BackendError> {
        let adapter = match backend.adapter {
            ConfirmedAdapter::Http(config) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| {
                        BackendError::new(BackendErrorKind::Failed, "could not start HTTP runtime")
                    })?;
                let client = OpenAiCompatClient::new(config).map_err(http_error)?;
                RunnerAdapter::Http { runtime, client }
            }
            ConfirmedAdapter::Command(command) => RunnerAdapter::Command(command),
        };
        Ok(Self { adapter, cancel })
    }

    pub(crate) fn complete(&mut self, system: &str, user: &str) -> Result<String, BackendError> {
        self.complete_messages(&[
            BackendMessage::new(MessageRole::System, system),
            BackendMessage::new(MessageRole::User, user),
        ])
    }

    pub(crate) fn complete_messages(
        &mut self,
        messages: &[BackendMessage],
    ) -> Result<String, BackendError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(BackendError::cancelled());
        }
        match &mut self.adapter {
            RunnerAdapter::Http { runtime, client } => {
                let chat = messages.iter().map(to_chat_message).collect::<Vec<_>>();
                let cancel = self.cancel;
                runtime.block_on(async {
                    tokio::select! {
                        result = client.complete_messages(&chat) => result.map_err(http_error),
                        () = wait_for_cancel(cancel) => Err(BackendError::cancelled()),
                    }
                })
            }
            RunnerAdapter::Command(command) => {
                super::command_adapter::complete(command, messages, self.cancel)
            }
        }
    }
}

pub(super) fn http_error(error: LlmError) -> BackendError {
    let kind = match &error {
        LlmError::Client(_) => BackendErrorKind::Failed,
        LlmError::InsecureApiKey { .. } => BackendErrorKind::Unavailable,
        LlmError::Request(_) | LlmError::Http { .. } => BackendErrorKind::Unreachable,
        LlmError::ResponseTooLarge => BackendErrorKind::OutputTooLarge,
        LlmError::InvalidResponse(_) | LlmError::MissingContent => BackendErrorKind::Incompatible,
    };
    BackendError::new(kind, error.to_string())
}

impl BackendMessage {
    pub(crate) fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

impl MessageRole {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

impl BackendError {
    pub(crate) fn new(kind: BackendErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: sanitize(message.into()),
        }
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(BackendErrorKind::Cancelled, "request cancelled")
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn to_chat_message(message: &BackendMessage) -> ChatMessage {
    match message.role {
        MessageRole::System => ChatMessage::system(&message.content),
        MessageRole::User => ChatMessage::user(&message.content),
        MessageRole::Assistant => ChatMessage::assistant(&message.content),
    }
}

fn sanitize(message: String) -> String {
    message
        .chars()
        .take(512)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_missing_credential_fails_after_confirmation_resolution() {
        let preset = crate::config::llm::parse(
            "[[llm.backends]]\nname='hosted'\ntype='openai-compatible'\nbase_url='https://models.example/v1'\nmodel='remote'\napi_key_env='CATOMIC_ISSUE_67_TEST_MISSING_KEY'\n",
        )
        .unwrap()
        .default_preset()
        .clone();
        let error = match ConfirmedBackend::resolve(&preset) {
            Ok(_) => panic!("missing explicit credential must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind, BackendErrorKind::Unavailable);
        assert!(error
            .to_string()
            .contains("CATOMIC_ISSUE_67_TEST_MISSING_KEY"));
    }

    #[test]
    fn legacy_optional_credential_remains_backward_compatible() {
        let preset = crate::config::llm::parse(
            "[llm]\nbase_url='http://127.0.0.1:9/v1'\nmodel='local'\napi_key_env='CATOMIC_ISSUE_67_TEST_MISSING_KEY'\n",
        )
        .unwrap()
        .default_preset()
        .clone();
        assert!(ConfirmedBackend::resolve(&preset).is_ok());
    }
}
