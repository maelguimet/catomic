//! Purpose: define prompt-command help and lookup metadata.
//! Owns: prompt-command names, aliases, and concise purposes.
//! Must not: duplicate configurable actions, dispatch commands, mutate state, or start services.
//! Invariants: configurable actions live only in `config::actions`; prompt aliases are unique.
//! Phase: post-v0.1 discoverability and help-drift prevention.

mod prompt_commands;
#[cfg(test)]
pub(crate) use prompt_commands::PROMPT_COMMANDS;
pub(crate) use prompt_commands::{prompt_command, PromptCommand};

#[cfg(test)]
mod tests;
