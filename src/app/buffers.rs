//! Purpose: this file must own multiple-buffer construction and switching.
//! Owns: inactive buffer slots, ring ordering, and per-buffer state swaps.
//! Must not: decode keys, render, mutate buffer content, or perform terminal I/O.
//! Invariants: active state stays in App; each inactive slot retains its file,
//!   cursor/buffer, watcher, viewport, message, and pending file operations.
//! Phase: 2-b multiple-buffer foundation.

use std::io;
use std::mem;

use crate::buffer::Buffer;
use crate::config::big_files::BigFileConfig;
use crate::file::watcher::FileWatcher;

use super::{command_prompt, completion, reload, save, search, selection, view, App, FileState};

pub(crate) struct BufferSlot {
    buffer: Box<dyn Buffer>,
    file: FileState,
    file_watcher: Option<FileWatcher>,
    message: Option<String>,
    pending_save_conflict: Option<save::PendingSaveConflict>,
    pending_reload: Option<reload::PendingReload>,
    search: search::SearchUiState,
    selection: selection::SelectionUiState,
    view: view::ViewOptions,
    scroll_top: usize,
    scroll_left: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BufferDirection {
    Next,
    Previous,
}

impl BufferSlot {
    fn from_app(app: App) -> Self {
        Self {
            buffer: app.buffer,
            file: app.file,
            file_watcher: app.file_watcher,
            message: app.message,
            pending_save_conflict: app.pending_save_conflict,
            pending_reload: app.pending_reload,
            search: app.search,
            selection: app.selection,
            view: app.view,
            scroll_top: app.screen.scroll_top,
            scroll_left: app.screen.scroll_left,
        }
    }

    fn swap_with_active(&mut self, app: &mut App) {
        mem::swap(&mut self.buffer, &mut app.buffer);
        mem::swap(&mut self.file, &mut app.file);
        mem::swap(&mut self.file_watcher, &mut app.file_watcher);
        mem::swap(&mut self.message, &mut app.message);
        mem::swap(
            &mut self.pending_save_conflict,
            &mut app.pending_save_conflict,
        );
        mem::swap(&mut self.pending_reload, &mut app.pending_reload);
        mem::swap(&mut self.search, &mut app.search);
        mem::swap(&mut self.selection, &mut app.selection);
        mem::swap(&mut self.view, &mut app.view);
        mem::swap(&mut self.scroll_top, &mut app.screen.scroll_top);
        mem::swap(&mut self.scroll_left, &mut app.screen.scroll_left);
    }
}

impl App {
    pub(crate) fn new_with_paths_and_big_file_config(
        initial_paths: &[String],
        big_files: BigFileConfig,
    ) -> io::Result<Self> {
        Self::new_with_paths_and_config(initial_paths, big_files, true)
    }

    pub(crate) fn new_with_paths_and_config(
        initial_paths: &[String],
        big_files: BigFileConfig,
        auto_reload: bool,
    ) -> io::Result<Self> {
        let first_path = initial_paths.first().map(String::as_str);
        let mut app = Self::new_with_config(first_path, big_files, auto_reload)?;
        for path in initial_paths.iter().skip(1) {
            let extra = Self::new_with_config(Some(path), big_files, auto_reload)?;
            app.inactive_buffers.push_back(BufferSlot::from_app(extra));
        }
        Ok(app)
    }

    pub(crate) fn buffer_count(&self) -> usize {
        self.inactive_buffers.len().saturating_add(1)
    }

    pub(crate) fn dirty_buffer_count(&self) -> usize {
        usize::from(self.file.dirty)
            + self
                .inactive_buffers
                .iter()
                .filter(|slot| slot.file.dirty)
                .count()
    }

    pub(crate) fn switch_buffer(&mut self, direction: BufferDirection) -> bool {
        if self.inactive_buffers.is_empty() {
            return false;
        }

        search::cancel_running_search(self);
        command_prompt::cancel_running_goto(self);
        if completion::cancel(self) {
            self.message = None;
        }
        if self.pending_quit_confirm {
            self.message = None;
            self.pending_quit_confirm = false;
        }
        let mut slot = match direction {
            BufferDirection::Next => self.inactive_buffers.pop_front(),
            BufferDirection::Previous => self.inactive_buffers.pop_back(),
        }
        .expect("non-empty inactive buffer ring");
        slot.swap_with_active(self);
        match direction {
            BufferDirection::Next => self.inactive_buffers.push_back(slot),
            BufferDirection::Previous => self.inactive_buffers.push_front(slot),
        }

        let count = self.buffer_count();
        self.active_buffer_index = match direction {
            BufferDirection::Next => self.active_buffer_index.saturating_add(1) % count,
            BufferDirection::Previous => self.active_buffer_index.saturating_add(count - 1) % count,
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    fn temp_file(label: &str, text: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "catomic_buffers_{label}_{}_{nonce}.txt",
            std::process::id()
        ));
        fs::write(&path, text).unwrap();
        path
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn multiple_paths_open_in_argument_order_and_wrap() {
        let first = temp_file("first", "alpha");
        let second = temp_file("second", "beta");
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        let mut app =
            App::new_with_paths_and_big_file_config(&paths, BigFileConfig::default()).unwrap();

        assert_eq!(app.buffer_count(), 2);
        assert_eq!(app.active_buffer_index, 0);
        assert_eq!(app.buffer.to_string(), "alpha");

        assert!(app.switch_buffer(BufferDirection::Next));
        assert_eq!(app.active_buffer_index, 1);
        assert_eq!(app.buffer.to_string(), "beta");

        assert!(app.switch_buffer(BufferDirection::Next));
        assert_eq!(app.active_buffer_index, 0);
        assert_eq!(app.buffer.to_string(), "alpha");

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn switching_preserves_edits_cursor_and_viewport_per_buffer() {
        let first = temp_file("state_first", "alpha");
        let second = temp_file("state_second", "beta");
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        let mut app =
            App::new_with_paths_and_big_file_config(&paths, BigFileConfig::default()).unwrap();

        app.buffer.move_right();
        app.buffer.insert_char('!');
        app.screen.scroll_top = 7;
        app.screen.scroll_left = 3;
        app.view.line_numbers = true;
        app.view.whitespace = true;
        app.file.dirty = true;

        app.switch_buffer(BufferDirection::Next);
        assert_eq!(app.buffer.to_string(), "beta");
        assert!(!app.file.dirty);
        assert_eq!(app.screen.scroll_top, 0);
        assert_eq!(app.screen.scroll_left, 0);
        assert!(!app.view.line_numbers);
        assert!(!app.view.whitespace);

        app.screen.scroll_top = 11;
        app.switch_buffer(BufferDirection::Previous);
        assert_eq!(app.buffer.to_string(), "a!lpha");
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 0, col: 2 }
        );
        assert!(app.file.dirty);
        assert_eq!(app.screen.scroll_top, 7);
        assert_eq!(app.screen.scroll_left, 3);
        assert!(app.view.line_numbers);
        assert!(app.view.whitespace);

        app.switch_buffer(BufferDirection::Next);
        assert_eq!(app.screen.scroll_top, 11);

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn switching_a_single_buffer_is_a_no_op() {
        let mut app = App::new(None).unwrap();
        app.screen.scroll_top = 9;

        assert!(!app.switch_buffer(BufferDirection::Next));
        assert_eq!(app.buffer_count(), 1);
        assert_eq!(app.active_buffer_index, 0);
        assert_eq!(app.screen.scroll_top, 9);
    }

    #[test]
    fn dirty_count_includes_inactive_buffers() {
        let first = temp_file("dirty_first", "alpha");
        let second = temp_file("dirty_second", "beta");
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        let mut app =
            App::new_with_paths_and_big_file_config(&paths, BigFileConfig::default()).unwrap();

        app.file.dirty = true;
        app.switch_buffer(BufferDirection::Next);
        assert_eq!(app.dirty_buffer_count(), 1);

        app.file.dirty = true;
        assert_eq!(app.dirty_buffer_count(), 2);

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn alt_page_keys_switch_buffers_and_render_active_position() {
        let first = temp_file("keys_first", "alpha");
        let second = temp_file("keys_second", "beta");
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        let mut app =
            App::new_with_paths_and_big_file_config(&paths, BigFileConfig::default()).unwrap();
        let mut out = Vec::new();

        app.handle_key_with(&mut out, key(KeyCode::PageDown, KeyModifiers::ALT))
            .unwrap();
        assert_eq!(app.buffer.to_string(), "beta");
        assert!(String::from_utf8_lossy(&out).contains("buffer 2/2"));

        app.handle_key_with(&mut out, key(KeyCode::PageUp, KeyModifiers::ALT))
            .unwrap();
        assert_eq!(app.buffer.to_string(), "alpha");
        assert_eq!(app.active_buffer_index, 0);

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn quit_guard_includes_a_dirty_inactive_buffer() {
        let first = temp_file("quit_first", "alpha");
        let second = temp_file("quit_second", "beta");
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        let mut app =
            App::new_with_paths_and_big_file_config(&paths, BigFileConfig::default()).unwrap();
        app.file.dirty = true;
        app.switch_buffer(BufferDirection::Next);
        let mut out = Vec::new();

        app.handle_key_with(&mut out, key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(!app.should_quit);
        assert!(app.pending_quit_confirm);
        assert!(app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Unsaved changes"));

        app.handle_key_with(&mut out, key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.should_quit);

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }
}
