//! Purpose: define every stable, user-facing shortcut action and its surface contract.
//! Owns: action names, labels, scopes, default chord inventory, and help formatting.
//! Must not: parse user TOML, dispatch App behavior, inspect terminal events, or mutate state.
//! Invariants: names are unique; every action has a scope and at least one default chord.
//! Phase: issue #62 complete shortcut customization.

use std::fmt::Write;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Scope {
    Global,
    Editor,
    Prompt,
    Search,
    Completion,
    Preview,
    Picker,
    Help,
}

impl Scope {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Editor => "editor",
            Self::Prompt => "prompt",
            Self::Search => "search",
            Self::Completion => "completion",
            Self::Preview => "preview",
            Self::Picker => "picker",
            Self::Help => "help",
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum InputKind {
    Keyboard,
    Mouse,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Action {
    Help,
    Quit,
    Save,
    SaveAs,
    Open,
    New,
    Close,
    Reload,
    Search,
    Replace,
    GotoLine,
    CommandPrompt,
    Complete,
    Undo,
    Redo,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    LineStart,
    LineEnd,
    SelectLineStart,
    SelectLineEnd,
    DocumentStart,
    DocumentEnd,
    SelectDocumentStart,
    SelectDocumentEnd,
    ViewportUp,
    ViewportDown,
    SelectViewportUp,
    SelectViewportDown,
    WordLeft,
    WordRight,
    SelectWordLeft,
    SelectWordRight,
    ParagraphPrevious,
    ParagraphNext,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    InsertNewline,
    Indent,
    Unindent,
    ToggleOverwrite,
    SelectAll,
    Copy,
    Cut,
    Paste,
    PreviousBuffer,
    NextBuffer,
    PreviousPage,
    NextPage,
    MarkdownPreview,
    LineNumbers,
    Whitespace,
    SoftWrap,
    PromptSubmit,
    PromptCancel,
    PromptDeleteBackward,
    SearchNext,
    SearchPrevious,
    SearchCancel,
    CompletionNext,
    CompletionPrevious,
    CompletionAccept,
    CompletionCancel,
    PreviewAccept,
    PreviewCancel,
    PickerAccept,
    PickerCancel,
    HelpClose,
    MousePlaceCursor,
    MouseExtendSelection,
    MouseFinishSelection,
    MouseSelectWord,
}

#[derive(Clone, Copy)]
pub(crate) struct Descriptor {
    pub(crate) action: Action,
    pub(crate) name: &'static str,
    pub(crate) label: &'static str,
    pub(crate) scopes: &'static [Scope],
    pub(crate) defaults: &'static [&'static str],
    pub(crate) input: InputKind,
}

mod registry;
pub(crate) use registry::REGISTRY;

pub(crate) fn descriptor(action: Action) -> &'static Descriptor {
    REGISTRY
        .iter()
        .find(|entry| entry.action == action)
        .expect("every Action must have a descriptor")
}

pub(crate) fn parse_action(name: &str) -> Option<Action> {
    let name = name.trim().to_ascii_lowercase();
    REGISTRY
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.action)
}

pub(crate) fn help_text() -> String {
    let mut text = String::from(
        "Catomic help - configurable actions and command quick reference\n\n\
         Actions and built-in default chords are listed below. [keybindings] can replace\n\
         or unbind them; restart Catomic after saving configuration changes.\n\
         Global actions take precedence, then the active local surface, then editor input.\n\
         Printable typing is never treated as a configurable shortcut.\n\n",
    );
    for entry in REGISTRY {
        let scopes = entry
            .scopes
            .iter()
            .map(|scope| scope.name())
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(
            text,
            "  {:<27} {:<24} {:<22} {}",
            entry.name,
            entry.defaults.join(" / "),
            format!("[{scopes}]"),
            entry.label
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_user_guide_inventory_matches_registry() {
        let mut names = std::collections::HashSet::new();
        let guide = include_str!("../../docs/user-guide.md");
        let inventory = guide
            .split_once("<!-- action-registry-start -->")
            .and_then(|(_, tail)| tail.split_once("<!-- action-registry-end -->"))
            .map(|(inventory, _)| inventory)
            .expect("user guide action registry markers")
            .trim()
            .strip_prefix("```text\n")
            .and_then(|inventory| inventory.strip_suffix("\n```"))
            .expect("user guide action registry text fence");
        for descriptor in REGISTRY {
            assert!(names.insert(descriptor.name), "duplicate action name");
        }
        assert_eq!(inventory, registry_reference());
    }

    fn registry_reference() -> String {
        REGISTRY
            .iter()
            .map(|descriptor| {
                format!(
                    "{} | {} | {}",
                    descriptor.name,
                    descriptor
                        .scopes
                        .iter()
                        .map(|scope| scope.name())
                        .collect::<Vec<_>>()
                        .join(","),
                    descriptor.defaults.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
