//! Purpose: decode named external commands into a bounded, explicit execution policy.
//! Owns: `[commands]` parsing, input/output modes, names, and timeout validation.
//! Must not: spawn processes, inspect buffers, dispatch hooks, or mutate files.
//! Invariants: commands are named, non-empty, time-bounded, and have valid output targets.
//! Phase: 7 external command configuration.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

const DEFAULT_TIMEOUT_SECS: u64 = 10;
const MAX_TIMEOUT_SECS: u64 = 300;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CommandInput {
    #[default]
    None,
    Selection,
    Buffer,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CommandOutput {
    #[default]
    Preview,
    Insert,
    ReplaceInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub(crate) command: String,
    pub(crate) input: CommandInput,
    pub(crate) output: CommandOutput,
    pub(crate) timeout: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CommandConfig {
    commands: BTreeMap<String, CommandSpec>,
}

impl CommandConfig {
    pub(crate) fn get(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.commands.len()
    }
}

pub(crate) fn parse(text: &str) -> io::Result<CommandConfig> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        commands: BTreeMap<String, RawCommandSpec>,
    }

    let raw = super::decode::<ConfigFile>(text)?;
    let mut commands = BTreeMap::new();
    for (name, raw_spec) in raw.commands {
        validate_name(&name)?;
        let spec = finish_spec(&name, raw_spec)?;
        commands.insert(name, spec);
    }
    Ok(CommandConfig { commands })
}

pub(crate) fn load_from(path: &Path) -> io::Result<CommandConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(CommandConfig::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<CommandConfig> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    match super::big_files::config_path(xdg.as_deref(), home.as_deref()) {
        Some(path) => load_from(&path),
        None => Ok(CommandConfig::default()),
    }
}

#[derive(Deserialize)]
struct RawCommandSpec {
    command: String,
    #[serde(default)]
    input: CommandInput,
    #[serde(default)]
    output: CommandOutput,
    #[serde(default = "default_timeout_secs")]
    timeout_secs: u64,
}

fn finish_spec(name: &str, raw: RawCommandSpec) -> io::Result<CommandSpec> {
    let command = raw.command.trim().to_string();
    if command.is_empty() {
        return Err(invalid(format!(
            "commands.{name}.command must not be empty"
        )));
    }
    if !(1..=MAX_TIMEOUT_SECS).contains(&raw.timeout_secs) {
        return Err(invalid(format!(
            "commands.{name}.timeout_secs must be between 1 and {MAX_TIMEOUT_SECS}"
        )));
    }
    if raw.output == CommandOutput::ReplaceInput && raw.input == CommandInput::None {
        return Err(invalid(format!(
            "commands.{name} cannot replace input when input is none"
        )));
    }
    Ok(CommandSpec {
        command,
        input: raw.input,
        output: raw.output,
        timeout: Duration::from_secs(raw.timeout_secs),
    })
}

fn validate_name(name: &str) -> io::Result<()> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(invalid(format!(
            "command name {name:?} must use ASCII letters, digits, '-' or '_'"
        )))
    }
}

const fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_configuration_has_no_commands() {
        assert_eq!(CommandConfig::default().len(), 0);
        assert!(parse("").unwrap().get("format").is_none());
    }

    #[test]
    fn parses_named_command_policy() {
        let config = parse(
            "[commands.upper]\ncommand = \"tr a-z A-Z\"\ninput = \"selection\"\n\
             output = \"replace-input\"\ntimeout_secs = 3\n",
        )
        .unwrap();
        let spec = config.get("upper").unwrap();

        assert_eq!(spec.command, "tr a-z A-Z");
        assert_eq!(spec.input, CommandInput::Selection);
        assert_eq!(spec.output, CommandOutput::ReplaceInput);
        assert_eq!(spec.timeout, Duration::from_secs(3));
    }

    #[test]
    fn defaults_to_no_input_read_only_preview_and_ten_seconds() {
        let config = parse("[commands.date]\ncommand = \"date +%F\"\n").unwrap();
        let spec = config.get("date").unwrap();

        assert_eq!(spec.input, CommandInput::None);
        assert_eq!(spec.output, CommandOutput::Preview);
        assert_eq!(spec.timeout, Duration::from_secs(10));
    }

    #[test]
    fn rejects_unsafe_names_empty_commands_and_invalid_policies() {
        for text in [
            "[commands.\"bad name\"]\ncommand = \"true\"\n",
            "[commands.empty]\ncommand = \"  \"\n",
            "[commands.fast]\ncommand = \"true\"\ntimeout_secs = 0\n",
            "[commands.slow]\ncommand = \"true\"\ntimeout_secs = 301\n",
            "[commands.bad]\ncommand = \"true\"\noutput = \"replace-input\"\n",
        ] {
            assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
