//! Purpose: define prompt-command help and lookup metadata.
//! Owns: prompt-command names, aliases, and concise purposes.
//! Must not: duplicate configurable actions, dispatch commands, mutate state, or start services.
//! Invariants: configurable actions live only in `config::actions`; prompt aliases are unique.
//! Phase: post-v0.1 discoverability and help-drift prevention.

mod prompt_commands;
pub(crate) use prompt_commands::{prompt_command, PromptCommand, PROMPT_COMMANDS};

#[cfg(test)]
mod tests;
