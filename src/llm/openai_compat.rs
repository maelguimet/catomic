//! Purpose: this file must perform one confirmed OpenAI-compatible chat request.
//! Owns: transient HTTP client construction, typed JSON, response bounds, and errors.
//! Must not: load config, collect context, persist clients, retry silently, or mutate files.
//! Invariants: clients exist only inside confirmed workers; response capture is bounded.
//! Phase: 6 (LLM, Powerful but Caged).

use std::time::Duration;

use serde::{Deserialize, Serialize};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub timeout: Duration,
}

#[derive(Debug)]
pub enum LlmError {
    Client(String),
    Request(String),
    Http { status: u16, body: String },
    ResponseTooLarge,
    InvalidResponse(String),
    MissingContent,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(error) => write!(formatter, "could not create HTTP client: {error}"),
            Self::Request(error) => write!(formatter, "request failed: {error}"),
            Self::Http { status, body } => {
                let summary: String = body.chars().take(200).collect();
                write!(formatter, "endpoint returned HTTP {status}: {summary}")
            }
            Self::ResponseTooLarge => write!(formatter, "endpoint response exceeded 2 MiB"),
            Self::InvalidResponse(error) => write!(formatter, "invalid endpoint JSON: {error}"),
            Self::MissingContent => write!(formatter, "endpoint response contained no message"),
        }
    }
}

pub struct OpenAiCompatClient {
    client: reqwest::Client,
    config: LlmConfig,
}

impl OpenAiCompatClient {
    pub fn new(config: LlmConfig) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| LlmError::Client(error.to_string()))?;
        Ok(Self { client, config })
    }

    pub async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        self.complete_messages(&[ChatMessage::system(system), ChatMessage::user(user)])
            .await
    }

    pub async fn complete_messages(&self, messages: &[ChatMessage]) -> Result<String, LlmError> {
        let endpoint = format!("{}/chat/completions", self.config.base_url);
        let request = ChatRequest {
            model: &self.config.model,
            messages,
        };
        let mut builder = self.client.post(endpoint).json(&request);
        if let Some(key) = self.config.api_key.as_deref() {
            builder = builder.bearer_auth(key);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| LlmError::Request(error.to_string()))?;
        let status = response.status();
        let body = read_bounded(response).await?;
        if !status.is_success() {
            return Err(LlmError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        let parsed: ChatResponse = serde_json::from_slice(&body)
            .map_err(|error| LlmError::InvalidResponse(error.to_string()))?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .filter(|content| !content.trim().is_empty())
            .ok_or(LlmError::MissingContent)
    }
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, LlmError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(LlmError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| LlmError::Request(error.to_string()))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(LlmError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
}

#[derive(Clone, Serialize)]
pub struct ChatMessage {
    role: &'static str,
    content: String,
}

impl ChatMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system",
            content: content.to_string(),
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user",
            content: content.to_string(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant",
            content: content.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[cfg(test)]
mod tests;
