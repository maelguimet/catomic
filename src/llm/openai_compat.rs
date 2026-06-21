//! OpenAI-compatible HTTP client (and local servers that speak the same format).
//!
//! - Configurable base URL + key
//! - Local-first friendly (ollama, llama.cpp server, etc.)
//! - Only used when `network_llm` capability allows.
//!
//! Never create an HTTP client for LLM in Plain mode at startup.

use std::time::Duration;

/// Configuration for talking to an OpenAI-compatible endpoint.
#[derive(Clone, Debug)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub timeout: Duration,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: None,
            model: "gpt-4o-mini".to_string(),
            timeout: Duration::from_secs(120),
        }
    }
}

/// Very thin client stub.
/// Real implementation will use reqwest (once added to Cargo.toml when needed).
pub struct OpenAiCompatClient {
    _config: LlmConfig,
}

impl OpenAiCompatClient {
    /// Only construct when network_llm capability is true and user has confirmed.
    pub fn new(config: LlmConfig) -> Self {
        Self { _config: config }
    }

    // TODO: chat_completions, with context budget, etc.
}
