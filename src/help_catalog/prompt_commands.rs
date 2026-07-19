//! Purpose: catalog every public prompt command and alias with concise user guidance.
//! Owns: prompt command identities, lookup spellings, context, and safety descriptions.
//! Must not: dispatch commands, construct App services, read configuration, or access network.
//! Invariants: the prompt parser and built-in reference consume this same catalog.
//! Phase: post-v0.1 discoverability and help-drift prevention.

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
    Project,
    Plain,
    Files,
    Lint,
    Diagnostics,
    DiagnosticNext,
    DiagnosticPrevious,
    Run,
    Recover,
    SelectModel,
    Meow,
    BigMeow,
    GitMeow,
    MegaMeow,
    Quit,
}

pub(crate) struct PromptCommandSpec {
    pub(crate) command: PromptCommand,
    pub(crate) names: &'static [&'static str],
    pub(crate) syntax: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) purpose: &'static str,
}

const fn command(
    command: PromptCommand,
    names: &'static [&'static str],
    syntax: &'static str,
    aliases: &'static [&'static str],
    purpose: &'static str,
) -> PromptCommandSpec {
    PromptCommandSpec {
        command,
        names,
        syntax,
        aliases,
        purpose,
    }
}

pub(crate) const PROMPT_COMMANDS: &[PromptCommandSpec] = &[
    command(PromptCommand::Help, &["help", "shortcuts"], "help", &["shortcuts"], "Open this read-only help."),
    command(PromptCommand::Save, &["save", "write", "w"], "save", &["write", "w"], "Save the active buffer through the normal conflict guard."),
    command(PromptCommand::SaveAs, &["save-as", "saveas"], "save as PATH", &["write as PATH", "w as PATH", "save-as PATH", "saveas PATH"], "Save to a new path; replacing an existing target requires confirmation."),
    command(PromptCommand::Open, &["open", "edit", "e"], "open PATH", &["edit PATH", "e PATH"], "Open a buffer; a missing path is not created until save."),
    command(PromptCommand::New, &["new"], "new", &[], "Create an untitled buffer."),
    command(PromptCommand::Close, &["close"], "close", &[], "Close the active buffer only when it has no unsaved edits."),
    command(PromptCommand::CloseDiscard, &["close!"], "close!", &[], "Discard active-buffer edits and close it without saving."),
    command(PromptCommand::Config, &["config"], "config", &[], "Open the resolved user configuration path, confirming safe template creation when missing."),
    command(PromptCommand::Goto, &["goto", "line"], "goto LINE", &["line LINE"], "Go to a 1-based line; a past-end line moves to the final line."),
    command(PromptCommand::Replace, &["replace"], "replace", &[], "Open the two-stage Replace Next prompt; the edit is undoable."),
    command(PromptCommand::ReplaceAll, &["replace-all", "replaceall"], "replace-all", &["replaceall"], "Replace all matches as one undoable edit; ordinary buffers only."),
    command(PromptCommand::Project, &["project", "code"], "project", &["code"], "Enter opt-in Project mode; tooling remains explicit and lazy."),
    command(PromptCommand::Plain, &["plain", "text"], "plain", &["text"], "Leave Project mode, stop Project tasks, and discard its services/cache."),
    command(PromptCommand::Files, &["files"], "files", &[], "In Project mode, run bounded file discovery and open its picker."),
    command(PromptCommand::Lint, &["lint"], "lint", &[], "In Project mode, run the configured linter for a saved active file."),
    command(PromptCommand::Diagnostics, &["diagnostics", "dlist"], "diagnostics", &["dlist"], "Open the most recent diagnostic list; Project tooling only."),
    command(PromptCommand::DiagnosticNext, &["dnext"], "dnext", &[], "Jump to the next Project diagnostic, opening a discovered file if needed."),
    command(PromptCommand::DiagnosticPrevious, &["dprev"], "dprev", &[], "Jump to the previous Project diagnostic, opening a discovered file if needed."),
    command(PromptCommand::Run, &["run"], "run NAME", &[], "Run a configured trusted /bin/sh command; it may affect outside data, and output previews before any buffer edit."),
    command(PromptCommand::Recover, &["recover"], "recover", &[], "Preview a newer .catnap; Enter applies an undoable buffer edit but never saves."),
    command(PromptCommand::SelectModel, &["model", "models", "select-model"], "model", &["models", "select-model"], "Open the searchable session model/backend picker without invoking or persisting a backend."),
    command(PromptCommand::Meow, &["meow"], "meow INSTRUCTION", &[], "Explicitly send the selection or instruction block after destination/context confirmation."),
    command(PromptCommand::BigMeow, &["bigmeow"], "bigmeow INSTRUCTION", &[], "Explicitly send the current ordinary file after confirmation."),
    command(PromptCommand::GitMeow, &["gitmeow"], "gitmeow INSTRUCTION", &[], "In Project mode, use focused bounded repository context after confirmation."),
    command(PromptCommand::MegaMeow, &["megameow"], "megameow INSTRUCTION", &[], "In Project mode, use broader bounded repository context after confirmation."),
    command(PromptCommand::Quit, &["quit", "q"], "quit", &["q"], "Use the normal guarded quit path; repeat to discard dirty buffers."),
];

pub(crate) fn prompt_command(name: &str) -> Option<PromptCommand> {
    PROMPT_COMMANDS
        .iter()
        .find(|spec| spec.names.contains(&name))
        .map(|spec| spec.command)
}
