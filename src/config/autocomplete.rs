//! Purpose: load bounded, opt-in inline autocomplete policy from TOML.
//! Owns: `[autocomplete]` defaults, typed parsing, validation, and config-path reads.
//! Must not: load LLM credentials/settings, construct clients, or contact endpoints.
//! Invariants: automatic sending is disabled by default; all work/context limits are bounded.
//! Phase: post-v0.1 opt-in inline autocomplete.

use std::io;
use std::time::Duration;

use serde::Deserialize;

const DEFAULT_IDLE_DEBOUNCE_MS: u64 = 750;
const DEFAULT_MINIMUM_PREFIX_LENGTH: usize = 20;
const DEFAULT_MAX_CONTEXT_BEFORE: usize = 2_048;
const DEFAULT_MAX_CONTEXT_AFTER: usize = 512;
const DEFAULT_MAX_GENERATED_TOKENS: u32 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutocompleteConfig {
    pub(crate) enabled: bool,
    pub(crate) idle_debounce: Duration,
    pub(crate) minimum_prefix_length: usize,
    pub(crate) max_context_before: usize,
    pub(crate) max_context_after: usize,
    pub(crate) max_generated_tokens: u32,
    pub(crate) model: Option<String>,
    pub(crate) allow_remote: bool,
}

impl Default for AutocompleteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_debounce: Duration::from_millis(DEFAULT_IDLE_DEBOUNCE_MS),
            minimum_prefix_length: DEFAULT_MINIMUM_PREFIX_LENGTH,
            max_context_before: DEFAULT_MAX_CONTEXT_BEFORE,
            max_context_after: DEFAULT_MAX_CONTEXT_AFTER,
            max_generated_tokens: DEFAULT_MAX_GENERATED_TOKENS,
            model: None,
            allow_remote: false,
        }
    }
}

pub(crate) fn parse(text: &str) -> io::Result<AutocompleteConfig> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        autocomplete: RawAutocompleteConfig,
    }

    #[derive(Default, Deserialize)]
    struct RawAutocompleteConfig {
        enabled: Option<bool>,
        idle_debounce_ms: Option<u64>,
        minimum_prefix_length: Option<usize>,
        max_context_before: Option<usize>,
        max_context_after: Option<usize>,
        max_generated_tokens: Option<u32>,
        model: Option<String>,
        allow_remote: Option<bool>,
    }

    let raw = super::decode::<ConfigFile>(text)?.autocomplete;
    let defaults = AutocompleteConfig::default();
    let mut config = AutocompleteConfig {
        enabled: raw.enabled.unwrap_or(defaults.enabled),
        idle_debounce: Duration::from_millis(
            raw.idle_debounce_ms.unwrap_or(DEFAULT_IDLE_DEBOUNCE_MS),
        ),
        minimum_prefix_length: raw
            .minimum_prefix_length
            .unwrap_or(DEFAULT_MINIMUM_PREFIX_LENGTH),
        max_context_before: raw.max_context_before.unwrap_or(DEFAULT_MAX_CONTEXT_BEFORE),
        max_context_after: raw.max_context_after.unwrap_or(DEFAULT_MAX_CONTEXT_AFTER),
        max_generated_tokens: raw
            .max_generated_tokens
            .unwrap_or(DEFAULT_MAX_GENERATED_TOKENS),
        model: raw.model,
        allow_remote: raw.allow_remote.unwrap_or(defaults.allow_remote),
    };
    validate(&mut config)?;
    Ok(config)
}

fn validate(config: &mut AutocompleteConfig) -> io::Result<()> {
    if !(100..=10_000).contains(&config.idle_debounce.as_millis()) {
        return Err(invalid(
            "autocomplete.idle_debounce_ms must be between 100 and 10000",
        ));
    }
    if !(1..=4_096).contains(&config.minimum_prefix_length) {
        return Err(invalid(
            "autocomplete.minimum_prefix_length must be between 1 and 4096",
        ));
    }
    if !(64..=65_536).contains(&config.max_context_before) {
        return Err(invalid(
            "autocomplete.max_context_before must be between 64 and 65536",
        ));
    }
    if config.max_context_after > 16_384 {
        return Err(invalid(
            "autocomplete.max_context_after must be between 0 and 16384",
        ));
    }
    if config.minimum_prefix_length > config.max_context_before {
        return Err(invalid(
            "autocomplete.minimum_prefix_length must not exceed max_context_before",
        ));
    }
    if !(1..=512).contains(&config.max_generated_tokens) {
        return Err(invalid(
            "autocomplete.max_generated_tokens must be between 1 and 512",
        ));
    }
    if let Some(model) = config.model.as_mut() {
        *model = model.trim().to_string();
        if model.is_empty() {
            return Err(invalid("autocomplete.model must not be empty"));
        }
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_disabled_local_only_and_bounded() {
        let config = AutocompleteConfig::default();
        assert!(!config.enabled);
        assert!(!config.allow_remote);
        assert_eq!(config.idle_debounce, Duration::from_millis(750));
        assert_eq!(config.max_context_before, 2_048);
        assert_eq!(config.max_context_after, 512);
        assert_eq!(config.max_generated_tokens, 64);
    }

    #[test]
    fn parses_all_documented_settings_and_trims_model() {
        let config = parse(
            "[autocomplete]\nenabled = true\nidle_debounce_ms = 300\n\
             minimum_prefix_length = 12\nmax_context_before = 1024\n\
             max_context_after = 128\nmax_generated_tokens = 40\n\
             model = \" cat-writer \"\nallow_remote = true\n",
        )
        .unwrap();
        assert!(config.enabled);
        assert_eq!(config.idle_debounce, Duration::from_millis(300));
        assert_eq!(config.minimum_prefix_length, 12);
        assert_eq!(config.max_context_before, 1_024);
        assert_eq!(config.max_context_after, 128);
        assert_eq!(config.max_generated_tokens, 40);
        assert_eq!(config.model.as_deref(), Some("cat-writer"));
        assert!(config.allow_remote);
    }

    #[test]
    fn rejects_unbounded_or_inconsistent_values() {
        for text in [
            "[autocomplete]\nidle_debounce_ms = 0\n",
            "[autocomplete]\nminimum_prefix_length = 0\n",
            "[autocomplete]\nmax_context_before = 63\n",
            "[autocomplete]\nmax_context_after = 16385\n",
            "[autocomplete]\nmax_generated_tokens = 513\n",
            "[autocomplete]\nminimum_prefix_length = 65\nmax_context_before = 64\n",
            "[autocomplete]\nmodel = \"  \"\n",
        ] {
            assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
