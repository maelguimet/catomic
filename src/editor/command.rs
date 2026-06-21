//! Editor commands.
//!
//! These represent *intent* ("delete word", "save", "goto line 42", "run :meow").
//! They are produced by keymap / command palette / colon commands and then
//! executed against the current App / Buffer.
//!
//! This layer sits above raw terminal input.

/// High-level editor command.
/// Phase 0 only has a few implicit ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    // Movement
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    GotoLine(usize),

    // Editing
    InsertChar(char),
    InsertNewline,
    DeleteBack,
    DeleteForward,
    DeleteWord,

    // File
    Save,
    SaveAs(String),
    Open(String),

    // Search / replace
    StartSearch,
    FindNext,

    // LLM (gated by capabilities)
    Meow,     // current selection/block (Plain-allowed when invoked)
    BigMeow,  // current file
    MegaMeow, // repo-aware (Project)

    // Mode
    SwitchToPlain,
    SwitchToProject,

    // Misc
    Quit,
    Undo,
    Redo,
}

// TODO: command execution dispatcher in app or a dedicated executor.
