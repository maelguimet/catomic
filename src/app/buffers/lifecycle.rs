//! Purpose: own explicit new-buffer and close-buffer lifecycle transitions.
//! Owns: blank-buffer construction, dirty-close refusal, and active-slot removal.
//! Must not: decode keys, render, write files, or bypass explicit discard requests.
//! Invariants: dirty buffers close only with force; closing the last buffer leaves one blank.
//! Phase: post-v0.1 core usability.

use std::io;

use super::{App, BufferDirection, BufferSlot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseBufferOutcome {
    Closed,
    Dirty,
}

impl App {
    pub(crate) fn new_file_buffer(&mut self) -> io::Result<()> {
        let new_buffer = Self::new_with_config(
            None,
            self.big_files,
            self.auto_reload,
            self.editor_config.clone(),
            self.keybindings.clone(),
            self.command_config.clone(),
            self.cat_config,
        )?;
        self.inactive_buffers
            .push_front(BufferSlot::from_app(new_buffer));
        self.switch_buffer(BufferDirection::Next);
        self.message = Some("New untitled buffer.".to_string());
        Ok(())
    }

    pub(crate) fn close_active_buffer(&mut self, force: bool) -> io::Result<CloseBufferOutcome> {
        if self.file.dirty && !force {
            self.message =
                Some("Buffer has unsaved changes. Save it or use close! to discard.".to_string());
            return Ok(CloseBufferOutcome::Dirty);
        }
        let replacement = if self.inactive_buffers.is_empty() {
            let blank = Self::new_with_config(
                None,
                self.big_files,
                self.auto_reload,
                self.editor_config.clone(),
                self.keybindings.clone(),
                self.command_config.clone(),
                self.cat_config,
            )?;
            BufferSlot::from_app(blank)
        } else {
            self.inactive_buffers
                .pop_front()
                .expect("non-empty inactive buffer ring")
        };
        let old_count = self.buffer_count();
        let mut replacement = replacement;
        replacement.swap_with_active(self);
        drop(replacement);
        let new_count = self.buffer_count();
        self.active_buffer_index = if old_count == 1 || self.active_buffer_index >= new_count {
            0
        } else {
            self.active_buffer_index
        };
        self.pending_quit_confirm = false;
        self.message = Some("Buffer closed.".to_string());
        Ok(CloseBufferOutcome::Closed)
    }
}
