//! Purpose: this file must perform one confirmed OpenAI-compatible chat request.
//! Owns: transient HTTP client construction, typed JSON, response bounds, and errors.
//! Must not: load config, collect context, persist clients, retry silently, or mutate files.
//! Invariants: clients exist only inside confirmed workers; response capture is bounded.

use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MODEL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: Vec<(String, String)>,
    pub has_secret_headers: bool,
    pub model: String,
    pub timeout: Duration,
}

#[derive(Debug)]
pub enum LlmError {
    Client(String),
    InsecureCredential { endpoint: String },
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
            Self::InsecureCredential { endpoint } => write!(
                formatter,
                "refusing to send credentials over plaintext HTTP to non-loopback endpoint {endpoint}; use HTTPS, remove the credentials, or use a loopback endpoint"
            ),
            Self::Request(error) => write!(formatter, "request failed: {error}"),
            Self::Http { status, body } => {
                let _ = body;
                write!(formatter, "endpoint returned HTTP {status} (response body suppressed)")
            }
            Self::ResponseTooLarge => write!(formatter, "endpoint response exceeded its limit"),
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
        reject_insecure_credentials(&config)?;
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|error| LlmError::Client(error.to_string()))?;
        Ok(Self { client, config })
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
        for (name, value) in &self.config.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| LlmError::Request(error.to_string()))?;
        let status = response.status();
        let body = read_bounded(response, MAX_RESPONSE_BYTES).await?;
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

    pub(crate) async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let endpoint = format!("{}/models", self.config.base_url);
        let mut builder = self.client.get(endpoint);
        if let Some(key) = self.config.api_key.as_deref() {
            builder = builder.bearer_auth(key);
        }
        for (name, value) in &self.config.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| LlmError::Request(error.to_string()))?;
        let status = response.status();
        let body = read_bounded(response, MAX_MODEL_RESPONSE_BYTES).await?;
        if !status.is_success() {
            return Err(LlmError::Http {
                status: status.as_u16(),
                body: String::new(),
            });
        }
        let parsed: ModelListResponse = serde_json::from_slice(&body)
            .map_err(|error| LlmError::InvalidResponse(error.to_string()))?;
        let mut models = Vec::new();
        let mut seen = HashSet::new();
        for entry in parsed.data {
            let model = crate::config::llm::validated_model(entry.id)
                .map_err(|_| LlmError::InvalidResponse("invalid model identifier".to_string()))?;
            if seen.insert(model.clone()) {
                models.push(model);
            }
        }
        Ok(models)
    }
}

fn reject_insecure_credentials(config: &LlmConfig) -> Result<(), LlmError> {
    if config.api_key.is_none() && !config.has_secret_headers {
        return Ok(());
    }
    let url = reqwest::Url::parse(&config.base_url)
        .map_err(|error| LlmError::Client(format!("invalid endpoint URL: {error}")))?;
    if url.scheme() == "http" && !url.host_str().is_some_and(is_loopback_host) {
        return Err(LlmError::InsecureCredential {
            endpoint: config.base_url.clone(),
        });
    }
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    unbracketed
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

async fn read_bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, LlmError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(LlmError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| LlmError::Request(error.to_string()))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
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

#[derive(Deserialize)]
struct ModelListResponse {
    data: Vec<ModelListEntry>,
}

#[derive(Deserialize)]
struct ModelListEntry {
    id: String,
}

#[cfg(test)]
mod tests;
