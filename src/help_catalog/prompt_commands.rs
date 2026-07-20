//! Purpose: catalog every public prompt command and alias for parser lookup.
//! Owns: prompt command identities and lookup spellings.
//! Must not: dispatch commands, construct App services, read configuration, or access network.
//! Invariants: every accepted public spelling maps to one semantic command.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptCommand {
    Help,
    Save,
    SaveAs,
    Open,
    New,
    Close,
    CloseDiscard,
    Config,
    Goto,
    Replace,
    ReplaceAll,
    Run,
    Recover,
    SelectModel,
    RunClanker,
    ClearClankerChanges,
    Meow,
    BigMeow,
    GitMeow,
    MegaMeow,
    Quit,
}

pub(crate) struct PromptCommandSpec {
    pub(crate) command: PromptCommand,
    pub(crate) names: &'static [&'static str],
}

const fn command(command: PromptCommand, names: &'static [&'static str]) -> PromptCommandSpec {
    PromptCommandSpec { command, names }
}

pub(crate) const PROMPT_COMMANDS: &[PromptCommandSpec] = &[
    command(PromptCommand::Help, &["help", "shortcuts"]),
    command(PromptCommand::Save, &["save", "write", "w"]),
    command(PromptCommand::SaveAs, &["save-as", "saveas"]),
    command(PromptCommand::Open, &["open", "edit", "e"]),
    command(PromptCommand::New, &["new"]),
    command(PromptCommand::Close, &["close"]),
    command(PromptCommand::CloseDiscard, &["close!"]),
    command(PromptCommand::Config, &["config"]),
    command(PromptCommand::Goto, &["goto", "line"]),
    command(PromptCommand::Replace, &["replace"]),
    command(PromptCommand::ReplaceAll, &["replace-all", "replaceall"]),
    command(PromptCommand::Run, &["run"]),
    command(PromptCommand::Recover, &["recover"]),
    command(
        PromptCommand::SelectModel,
        &["model", "models", "select-model"],
    ),
    command(PromptCommand::RunClanker, &["run-clanker", "inline-meow"]),
    command(
        PromptCommand::ClearClankerChanges,
        &["clear-clanker-changes"],
    ),
    command(PromptCommand::Meow, &["meow"]),
    command(PromptCommand::BigMeow, &["bigmeow"]),
    command(PromptCommand::GitMeow, &["gitmeow"]),
    command(PromptCommand::MegaMeow, &["megameow"]),
    command(PromptCommand::Quit, &["quit", "q"]),
];

pub(crate) fn prompt_command(name: &str) -> Option<PromptCommand> {
    PROMPT_COMMANDS
        .iter()
        .find(|spec| spec.names.contains(&name))
        .map(|spec| spec.command)
}
