//! Patch parsing, preview, and application.
//!
//! All LLM output that wants to edit files must come through here.
//! Strong preference: unified diff / patch first.
//! Always: show preview, require explicit confirmation, result must be undoable.
//!
//! Golden tests for patch application are mandatory.

#[derive(Clone, Debug)]
pub struct Patch {
    // TODO: hunks, etc.
    pub raw: String,
}

impl Patch {
    pub fn from_llm_output(text: &str) -> Option<Self> {
        // Very naive: later use a proper diff parser.
        if text.contains("diff --git") || text.contains("@@") {
            Some(Patch { raw: text.to_string() })
        } else {
            None
        }
    }

    /// Apply to a buffer (or multiple files) and return the result.
    /// Must be previewable before commit.
    pub fn apply_preview(&self, _current: &str) -> Option<String> {
        // TODO: real application
        None
    }
}
