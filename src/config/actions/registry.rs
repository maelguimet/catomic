//! Purpose: declare the checked shortcut action inventory in one reviewable table.
//! Owns: stable names, labels, scopes, default chords, and keyboard/mouse classification.
//! Must not: contain parsing, dispatch, rendering, configuration IO, or tests.
//! Invariants: same-scope defaults do not collide after normalization.
//! Phase: issue #62 complete shortcut customization.

use super::{Descriptor, InputKind::*, Scope::*};

const E: &[super::Scope] = &[Editor];
const G: &[super::Scope] = &[Global];
const P: &[super::Scope] = &[Prompt];
const S: &[super::Scope] = &[Search];
const C: &[super::Scope] = &[Completion];
const V: &[super::Scope] = &[Preview];
const K: &[super::Scope] = &[Picker];
const H: &[super::Scope] = &[Help];
const EV: &[super::Scope] = &[Editor, Preview];
const NAV: &[super::Scope] = &[Editor, Preview, Picker, Help];
const PS: &[super::Scope] = &[Prompt, Search];

macro_rules! key {
    ($action:ident, $name:literal, $label:literal, $scopes:expr, [$($default:literal),+]) => {
        Descriptor { action: super::Action::$action, name: $name, label: $label, scopes: $scopes,
            defaults: &[$($default),+], input: Keyboard }
    };
}
macro_rules! mouse {
    ($action:ident, $name:literal, $label:literal, [$($default:literal),+]) => {
        Descriptor { action: super::Action::$action, name: $name, label: $label, scopes: E,
            defaults: &[$($default),+], input: Mouse }
    };
}

pub(crate) const REGISTRY: &[Descriptor] = &[
    key!(Help, "help", "Toggle shortcut help", G, ["ctrl+h", "f1"]),
    key!(Quit, "quit", "Guarded application quit", G, ["ctrl+q"]),
    key!(Save, "save", "Save active buffer", E, ["ctrl+s"]),
    key!(
        SaveAs,
        "save-as",
        "Open Save As prompt",
        E,
        ["ctrl+shift+s"]
    ),
    key!(Open, "open", "Open file prompt", E, ["ctrl+o"]),
    key!(New, "new", "New untitled buffer", E, ["ctrl+n"]),
    key!(Close, "close", "Close active clean buffer", E, ["ctrl+w"]),
    key!(
        Reload,
        "reload",
        "Check or confirm external reload",
        E,
        ["ctrl+r"]
    ),
    key!(Search, "search", "Open incremental search", E, ["ctrl+f"]),
    key!(
        Replace,
        "replace",
        "Open Replace Next prompt",
        E,
        ["ctrl+shift+f"]
    ),
    key!(
        GotoLine,
        "goto-line",
        "Open goto-line prompt",
        E,
        ["ctrl+g"]
    ),
    key!(
        CommandPrompt,
        "command-prompt",
        "Open command prompt",
        E,
        ["ctrl+shift+p", "f2"]
    ),
    key!(
        Complete,
        "complete",
        "Open local completion",
        E,
        ["ctrl+space"]
    ),
    key!(Undo, "undo", "Undo one edit", E, ["ctrl+z"]),
    key!(Redo, "redo", "Redo one edit", E, ["ctrl+y", "ctrl+shift+z"]),
    key!(MoveLeft, "move-left", "Move left", NAV, ["left"]),
    key!(MoveRight, "move-right", "Move right", NAV, ["right"]),
    key!(MoveUp, "move-up", "Move up", NAV, ["up"]),
    key!(MoveDown, "move-down", "Move down", NAV, ["down"]),
    key!(
        SelectLeft,
        "select-left",
        "Extend selection left",
        E,
        ["shift+left"]
    ),
    key!(
        SelectRight,
        "select-right",
        "Extend selection right",
        E,
        ["shift+right"]
    ),
    key!(
        SelectUp,
        "select-up",
        "Extend selection up",
        E,
        ["shift+up"]
    ),
    key!(
        SelectDown,
        "select-down",
        "Extend selection down",
        E,
        ["shift+down"]
    ),
    key!(LineStart, "line-start", "Move to line start", NAV, ["home"]),
    key!(LineEnd, "line-end", "Move to line end", NAV, ["end"]),
    key!(
        SelectLineStart,
        "select-line-start",
        "Select to line start",
        E,
        ["shift+home"]
    ),
    key!(
        SelectLineEnd,
        "select-line-end",
        "Select to line end",
        E,
        ["shift+end"]
    ),
    key!(
        DocumentStart,
        "document-start",
        "Move to document start",
        E,
        ["ctrl+home"]
    ),
    key!(
        DocumentEnd,
        "document-end",
        "Move to document end",
        E,
        ["ctrl+end"]
    ),
    key!(
        SelectDocumentStart,
        "select-document-start",
        "Select to document start",
        E,
        ["ctrl+shift+home"]
    ),
    key!(
        SelectDocumentEnd,
        "select-document-end",
        "Select to document end",
        E,
        ["ctrl+shift+end"]
    ),
    key!(
        ViewportUp,
        "viewport-up",
        "Move one viewport up",
        NAV,
        ["pageup"]
    ),
    key!(
        ViewportDown,
        "viewport-down",
        "Move one viewport down",
        NAV,
        ["pagedown"]
    ),
    key!(
        SelectViewportUp,
        "select-viewport-up",
        "Select one viewport up",
        E,
        ["shift+pageup"]
    ),
    key!(
        SelectViewportDown,
        "select-viewport-down",
        "Select one viewport down",
        E,
        ["shift+pagedown"]
    ),
    key!(
        WordLeft,
        "word-left",
        "Move one word left",
        E,
        ["ctrl+left"]
    ),
    key!(
        WordRight,
        "word-right",
        "Move one word right",
        E,
        ["ctrl+right"]
    ),
    key!(
        SelectWordLeft,
        "select-word-left",
        "Select one word left",
        E,
        ["ctrl+shift+left"]
    ),
    key!(
        SelectWordRight,
        "select-word-right",
        "Select one word right",
        E,
        ["ctrl+shift+right"]
    ),
    key!(
        ParagraphPrevious,
        "paragraph-previous",
        "Move to previous paragraph",
        E,
        ["ctrl+up"]
    ),
    key!(
        ParagraphNext,
        "paragraph-next",
        "Move to next paragraph",
        E,
        ["ctrl+down"]
    ),
    key!(
        DeleteBackward,
        "delete-backward",
        "Delete previous grapheme",
        E,
        ["backspace"]
    ),
    key!(
        DeleteForward,
        "delete-forward",
        "Delete next grapheme",
        E,
        ["delete"]
    ),
    key!(
        DeleteWordBackward,
        "delete-word-backward",
        "Delete previous word",
        E,
        ["ctrl+backspace"]
    ),
    key!(
        DeleteWordForward,
        "delete-word-forward",
        "Delete next word",
        E,
        ["ctrl+delete"]
    ),
    key!(
        InsertNewline,
        "insert-newline",
        "Insert an indented newline",
        E,
        ["enter"]
    ),
    key!(Indent, "indent", "Indent or insert to tab stop", E, ["tab"]),
    key!(
        Unindent,
        "unindent",
        "Unindent current or selected lines",
        E,
        ["shift+tab"]
    ),
    key!(
        ToggleOverwrite,
        "toggle-overwrite",
        "Toggle insert/overwrite mode",
        E,
        ["insert"]
    ),
    key!(
        SelectAll,
        "select-all",
        "Select active buffer or page",
        E,
        ["ctrl+a"]
    ),
    key!(Copy, "copy", "Copy selection", E, ["ctrl+c"]),
    key!(Cut, "cut", "Cut selection", E, ["ctrl+x"]),
    key!(Paste, "paste", "Paste internal clipboard", E, ["ctrl+v"]),
    key!(
        PreviousBuffer,
        "previous-buffer",
        "Switch to previous buffer",
        E,
        ["alt+pageup"]
    ),
    key!(
        NextBuffer,
        "next-buffer",
        "Switch to next buffer",
        E,
        ["alt+pagedown"]
    ),
    key!(
        PreviousPage,
        "previous-page",
        "Open previous large-file page",
        E,
        ["ctrl+pageup"]
    ),
    key!(
        NextPage,
        "next-page",
        "Open next large-file page",
        E,
        ["ctrl+pagedown"]
    ),
    key!(
        MarkdownPreview,
        "markdown-preview",
        "Toggle Markdown preview",
        EV,
        ["f6"]
    ),
    key!(
        LineNumbers,
        "line-numbers",
        "Toggle line numbers",
        E,
        ["f7"]
    ),
    key!(
        Whitespace,
        "whitespace",
        "Toggle visible whitespace",
        E,
        ["f8"]
    ),
    key!(SoftWrap, "soft-wrap", "Toggle soft wrapping", E, ["f9"]),
    key!(
        PromptSubmit,
        "prompt-submit",
        "Submit active prompt",
        P,
        ["enter"]
    ),
    key!(
        PromptCancel,
        "prompt-cancel",
        "Cancel active prompt",
        P,
        ["esc"]
    ),
    key!(
        PromptDeleteBackward,
        "prompt-delete-backward",
        "Delete prompt character",
        PS,
        ["backspace"]
    ),
    key!(
        SearchNext,
        "search-next",
        "Move to next search match",
        S,
        ["enter", "down"]
    ),
    key!(
        SearchPrevious,
        "search-previous",
        "Move to previous search match",
        S,
        ["up"]
    ),
    key!(
        SearchCancel,
        "search-cancel",
        "Close search and clear highlight",
        S,
        ["esc"]
    ),
    key!(
        CompletionNext,
        "completion-next",
        "Select next completion",
        C,
        ["tab", "ctrl+space"]
    ),
    key!(
        CompletionPrevious,
        "completion-previous",
        "Select previous completion",
        C,
        ["shift+tab"]
    ),
    key!(
        CompletionAccept,
        "completion-accept",
        "Accept completion",
        C,
        ["enter"]
    ),
    key!(
        CompletionCancel,
        "completion-cancel",
        "Dismiss completion",
        C,
        ["esc"]
    ),
    key!(
        PreviewAccept,
        "preview-accept",
        "Accept or apply preview",
        V,
        ["enter"]
    ),
    key!(
        PreviewCancel,
        "preview-cancel",
        "Cancel or close preview",
        V,
        ["esc"]
    ),
    key!(
        PickerAccept,
        "picker-accept",
        "Open picker selection",
        K,
        ["enter"]
    ),
    key!(
        PickerCancel,
        "picker-cancel",
        "Cancel or close picker",
        K,
        ["esc"]
    ),
    key!(HelpClose, "help-close", "Close shortcut help", H, ["esc"]),
    mouse!(
        MousePlaceCursor,
        "mouse-place-cursor",
        "Place cursor",
        ["mouse-left"]
    ),
    mouse!(
        MouseExtendSelection,
        "mouse-extend-selection",
        "Extend dragged selection",
        ["mouse-left-drag"]
    ),
    mouse!(
        MouseFinishSelection,
        "mouse-finish-selection",
        "Finish dragged selection",
        ["mouse-left-up"]
    ),
    mouse!(
        MouseSelectWord,
        "mouse-select-word",
        "Select double-clicked word",
        ["mouse-left-double"]
    ),
];
