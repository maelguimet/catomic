//! Purpose: decode named external commands and lifecycle hook references.
//! Owns: command policies, hook order, names, timeout validation, and reference validation.
//! Must not: spawn processes, inspect buffers, dispatch lifecycle events, or mutate files.
//! Invariants: commands are bounded; hooks reference unique commands defined in the same file.
//! Phase: 7 external command configuration.

use std::collections::BTreeMap;
use std::io;
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
    hooks: Hooks,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Hooks {
    on_open: Vec<String>,
    on_save: Vec<String>,
    before_llm: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HookEvent {
    Open,
    Save,
    BeforeLlm,
}

impl CommandConfig {
    pub(crate) fn get(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name)
    }

    pub(crate) fn hooks_for(&self, event: HookEvent) -> &[String] {
        match event {
            HookEvent::Open => &self.hooks.on_open,
            HookEvent::Save => &self.hooks.on_save,
            HookEvent::BeforeLlm => &self.hooks.before_llm,
        }
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
        #[serde(default)]
        hooks: RawHooks,
    }

    let raw = super::decode::<ConfigFile>(text)?;
    let mut commands = BTreeMap::new();
    for (name, raw_spec) in raw.commands {
        validate_name(&name)?;
        let spec = finish_spec(&name, raw_spec)?;
        commands.insert(name, spec);
    }
    let hooks = raw.hooks.into();
    validate_hooks(&commands, &hooks)?;
    Ok(CommandConfig { commands, hooks })
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

#[derive(Default, Deserialize)]
struct RawHooks {
    #[serde(default)]
    on_open: Vec<String>,
    #[serde(default)]
    on_save: Vec<String>,
    #[serde(default)]
    before_llm: Vec<String>,
}

impl From<RawHooks> for Hooks {
    fn from(raw: RawHooks) -> Self {
        Self {
            on_open: raw.on_open,
            on_save: raw.on_save,
            before_llm: raw.before_llm,
        }
    }
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

fn validate_hooks(commands: &BTreeMap<String, CommandSpec>, hooks: &Hooks) -> io::Result<()> {
    for (event, names) in [
        ("on_open", &hooks.on_open),
        ("on_save", &hooks.on_save),
        ("before_llm", &hooks.before_llm),
    ] {
        let mut seen = std::collections::BTreeSet::new();
        for name in names {
            if !commands.contains_key(name) {
                return Err(invalid(format!(
                    "hooks.{event} references unknown command {name:?}"
                )));
            }
            if !seen.insert(name) {
                return Err(invalid(format!(
                    "hooks.{event} contains duplicate command {name:?}"
                )));
            }
        }
    }
    Ok(())
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

    #[test]
    fn parses_ordered_hooks_that_reference_named_commands() {
        let config = parse(
            "[commands.first]\ncommand = \"true\"\n[commands.second]\ncommand = \"true\"\n\
             [hooks]\non_open = [\"first\", \"second\"]\non_save = [\"second\"]\n\
             before_llm = [\"first\"]\n",
        )
        .unwrap();

        assert_eq!(
            config.hooks_for(HookEvent::Open),
            &["first".to_string(), "second".to_string()]
        );
        assert_eq!(config.hooks_for(HookEvent::Save), &["second".to_string()]);
        assert_eq!(
            config.hooks_for(HookEvent::BeforeLlm),
            &["first".to_string()]
        );
    }

    #[test]
    fn rejects_unknown_and_duplicate_hook_commands() {
        for text in [
            "[hooks]\non_open = [\"missing\"]\n",
            "[commands.ok]\ncommand = \"true\"\n[hooks]\non_save = [\"ok\", \"ok\"]\n",
        ] {
            assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
