//! Purpose: this file must own multiple-buffer construction and switching.
//! Owns: inactive buffer slots, ring ordering, and per-buffer state swaps.
//! Must not: decode keys, render, mutate buffer content, or perform terminal I/O.
//! Invariants: active state stays in App; each inactive slot retains its file,
//!   cursor/buffer, watcher, viewport, message, and pending file operations.
//! Phase: 2-b multiple-buffer foundation.

use std::io;
use std::mem;
use std::path::Path;

use crate::buffer::Buffer;
#[cfg(test)]
use crate::config::big_files::BigFileConfig;
use crate::file::identity::BufferFileIdentity;
use crate::file::watcher::FileWatcher;

use super::{
    command_prompt, completion, external_command, hooks, inline_clanker, lint, llm_answer,
    llm_preview, llm_request, model_picker, project_files, recovery, reload, repo_llm, save,
    search, selection, view, App, FileState, StartupConfig,
};

mod lifecycle;

pub(crate) struct BufferSlot {
    buffer: Box<dyn Buffer>,
    file: FileState,
    file_watcher: Option<FileWatcher>,
    message: Option<String>,
    pending_save_conflict: Option<save::PendingSaveConflict>,
    pending_reload: Option<reload::PendingReload>,
    search: search::SearchUiState,
    recovery: recovery::RecoveryState,
    selection: selection::SelectionUiState,
    view: view::ViewOptions,
    clanker_changes: inline_clanker::ChangeHistory,
    scroll_top: usize,
    scroll_left: usize,
    wrap_col: usize,
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
            recovery: app.recovery,
            selection: app.selection,
            view: app.view,
            clanker_changes: app.clanker_changes,
            scroll_top: app.screen.scroll_top,
            scroll_left: app.screen.scroll_left,
            wrap_col: app.screen.wrap_col,
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
        mem::swap(&mut self.recovery, &mut app.recovery);
        mem::swap(&mut self.selection, &mut app.selection);
        mem::swap(&mut self.view, &mut app.view);
        mem::swap(&mut self.clanker_changes, &mut app.clanker_changes);
        mem::swap(&mut self.scroll_top, &mut app.screen.scroll_top);
        mem::swap(&mut self.scroll_left, &mut app.screen.scroll_left);
        mem::swap(&mut self.wrap_col, &mut app.screen.wrap_col);
    }
}

impl App {
    #[cfg(test)]
    pub(crate) fn new_with_paths_and_big_file_config(
        initial_paths: &[String],
        big_files: BigFileConfig,
    ) -> io::Result<Self> {
        Self::new_with_paths_and_config(
            initial_paths,
            StartupConfig {
                big_files,
                ..StartupConfig::default()
            },
        )
    }

    #[cfg(test)]
    pub(super) fn new_with_paths_and_config(
        initial_paths: &[String],
        config: StartupConfig,
    ) -> io::Result<Self> {
        let first_path = initial_paths.first().map(String::as_str);
        let mut app = Self::new_with_config(first_path, config.clone())?;
        for path in initial_paths.iter().skip(1) {
            let identity = BufferFileIdentity::from_path(Path::new(path))?;
            if app.contains_buffer_identity(&identity)? {
                continue;
            }
            let path = identity.open_path().to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "file path is not valid UTF-8")
            })?;
            let extra = Self::new_with_config(Some(path), config.clone())?;
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

        super::autocomplete::invalidate(self);
        search::cancel_running_search(self);
        command_prompt::cancel_running_goto(self);
        if completion::cancel(self) {
            self.message = None;
        }
        lint::close_view(self);
        project_files::close_view(self);
        model_picker::close(self);
        if llm_preview::close(self) {
            self.message = None;
        }
        if llm_answer::close(self) {
            self.message = None;
        }
        llm_request::cancel_all(self);
        inline_clanker::cancel_all(self);
        repo_llm::cancel_all(self);
        external_command::cancel_all(self);
        hooks::cancel_all(self);
        if recovery::close(self) {
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

    pub(crate) fn open_file_buffer(&mut self, path: &Path) -> io::Result<bool> {
        let target = BufferFileIdentity::from_path(path)?;
        if self.active_buffer_matches_identity(&target)? {
            return Ok(false);
        }
        if let Some(position) = self.inactive_buffer_position_for_identity(&target)? {
            for _ in 0..=position {
                self.switch_buffer(BufferDirection::Next);
            }
            return Ok(true);
        }
        let path = target.open_path().to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "file path is not valid UTF-8")
        })?;
        let opened = Self::new_with_config(Some(path), StartupConfig::for_new_buffer(self))?;
        self.inactive_buffers
            .push_front(BufferSlot::from_app(opened));
        let switched = self.switch_buffer(BufferDirection::Next);
        debug_assert!(switched, "new buffer must be switchable");
        hooks::trigger_open(self);
        Ok(true)
    }

    /// Save and Save As use the same live comparison as open. This catches two
    /// previously distinct missing paths that later converge through a symlink
    /// or filesystem replacement and blocks watcher-mediated alias overwrites.
    pub(crate) fn another_buffer_represents_path(&self, path: &Path) -> io::Result<bool> {
        let target = BufferFileIdentity::from_path(path)?;
        for slot in &self.inactive_buffers {
            let Some(path) = slot.file.path.as_deref() else {
                continue;
            };
            if BufferFileIdentity::from_path(path)?.matches(&target) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn replace_active_file_buffer(&mut self, path: &Path) -> io::Result<()> {
        let path = path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "file path is not valid UTF-8")
        })?;
        let opened = Self::new_with_config(Some(path), StartupConfig::for_new_buffer(self))?;
        let mut replacement = BufferSlot::from_app(opened);
        replacement.swap_with_active(self);
        hooks::trigger_open(self);
        Ok(())
    }

    #[cfg(test)]
    fn contains_buffer_identity(&self, target: &BufferFileIdentity) -> io::Result<bool> {
        if self.active_buffer_matches_identity(target)? {
            return Ok(true);
        }
        Ok(self
            .inactive_buffer_position_for_identity(target)?
            .is_some())
    }

    fn active_buffer_matches_identity(&self, target: &BufferFileIdentity) -> io::Result<bool> {
        let Some(path) = self.file.path.as_deref() else {
            return Ok(false);
        };
        Ok(BufferFileIdentity::from_path(path)?.matches(target))
    }

    fn inactive_buffer_position_for_identity(
        &self,
        target: &BufferFileIdentity,
    ) -> io::Result<Option<usize>> {
        for (position, slot) in self.inactive_buffers.iter().enumerate() {
            let Some(path) = slot.file.path.as_deref() else {
                continue;
            };
            if BufferFileIdentity::from_path(path)?.matches(target) {
                return Ok(Some(position));
            }
        }
        Ok(None)
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

    fn temp_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "catomic_buffers_{label}_{}_{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
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
    fn switching_preserves_buffer_state_and_keeps_f7_session_global() {
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
        let mut out = Vec::new();
        app.handle_key_with(&mut out, key(KeyCode::F(7), KeyModifiers::NONE))
            .unwrap();
        app.screen.scroll_top = 7;
        app.screen.scroll_left = 3;
        app.screen.wrap_col = 2;
        app.view.whitespace = true;
        app.view.soft_wrap = true;
        app.file.dirty = true;

        app.switch_buffer(BufferDirection::Next);
        assert_eq!(app.buffer.to_string(), "beta");
        assert!(!app.file.dirty);
        assert_eq!(app.screen.scroll_top, 0);
        assert_eq!(app.screen.scroll_left, 0);
        assert_eq!(app.screen.wrap_col, 0);
        assert!(app.view_preferences.line_numbers());
        assert!(!app.view.whitespace);
        assert!(!app.view.soft_wrap);

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
        assert_eq!(app.screen.wrap_col, 2);
        assert!(app.view_preferences.line_numbers());
        assert!(app.view.whitespace);
        assert!(app.view.soft_wrap);

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
        assert!(String::from_utf8_lossy(&out).contains("file 2/2"));
        assert!(String::from_utf8_lossy(&out).contains(&format!(
            "\x1b]0;{}\x07",
            second.file_name().unwrap().to_string_lossy()
        )));

        out.clear();
        app.handle_key_with(&mut out, key(KeyCode::PageUp, KeyModifiers::ALT))
            .unwrap();
        assert_eq!(app.buffer.to_string(), "alpha");
        assert_eq!(app.active_buffer_index, 0);
        assert!(String::from_utf8_lossy(&out).contains(&format!(
            "\x1b]0;{}\x07",
            first.file_name().unwrap().to_string_lossy()
        )));

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn terminal_safe_word_selection_fallback_stays_in_the_active_buffer() {
        let first = temp_file("selection_first", "one two");
        let second = temp_file("selection_second", "three four");
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        let mut app =
            App::new_with_paths_and_big_file_config(&paths, BigFileConfig::default()).unwrap();
        let mut out = Vec::new();

        app.handle_key_with(
            &mut out,
            key(KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT),
        )
        .unwrap();

        assert_eq!(app.active_buffer_index, 0);
        assert_eq!(app.buffer.to_string(), "one two");
        assert_eq!(
            app.selection.active().unwrap().ordered(),
            (
                crate::buffer::Cursor { row: 0, col: 0 },
                crate::buffer::Cursor { row: 0, col: 4 },
            )
        );

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

    #[test]
    fn open_file_buffer_reuses_paths_and_preserves_dirty_buffers() {
        let first = temp_file("open_first", "alpha");
        let second = temp_file("open_second", "beta");
        let mut app = App::new(first.to_str()).unwrap();
        app.buffer.insert_char('!');
        app.file.dirty = true;

        assert!(app.open_file_buffer(&second).unwrap());
        assert_eq!(app.buffer.to_string(), "beta");
        assert_eq!(app.buffer_count(), 2);
        assert!(!app.open_file_buffer(&second).unwrap());
        assert_eq!(app.buffer_count(), 2);

        assert!(app.open_file_buffer(&first).unwrap());
        assert_eq!(app.buffer.to_string(), "!alpha");
        assert!(app.file.dirty);
        assert_eq!(app.buffer_count(), 2);

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn relative_dot_and_absolute_spellings_reuse_the_first_buffer() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file_name = format!(
            "catomic_buffers_relative_{}_{}.txt",
            std::process::id(),
            nonce
        );
        let relative = PathBuf::from(&file_name);
        let dotted = PathBuf::from(".").join(&file_name);
        let absolute = std::env::current_dir().unwrap().join(&file_name);
        fs::write(&relative, "alpha").unwrap();
        let mut app = App::new(relative.to_str()).unwrap();

        assert!(!app.open_file_buffer(&dotted).unwrap());
        assert!(!app.open_file_buffer(&absolute).unwrap());
        assert_eq!(app.buffer_count(), 1);
        assert_eq!(app.file.path.as_deref(), Some(relative.as_path()));

        fs::remove_file(relative).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_open_reuses_the_referent_buffer_and_keeps_first_spelling() {
        use std::os::unix::fs::symlink;

        let root = temp_directory("open_symlink");
        let target = root.join("target.txt");
        let link = root.join("link.txt");
        fs::write(&target, "alpha").unwrap();
        symlink(&target, &link).unwrap();
        let mut app = App::new(link.to_str()).unwrap();

        assert!(!app.open_file_buffer(&target).unwrap());
        assert_eq!(app.buffer_count(), 1);
        assert_eq!(app.file.path.as_deref(), Some(link.as_path()));

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_open_reuses_one_buffer_identity() {
        let root = temp_directory("open_hard_link");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        fs::write(&first, "alpha").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let mut app = App::new(first.to_str()).unwrap();

        assert!(!app.open_file_buffer(&second).unwrap());
        assert_eq!(app.buffer_count(), 1);
        assert_eq!(app.file.path.as_deref(), Some(first.as_path()));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn distinct_missing_paths_remain_distinct_buffers() {
        let root = temp_directory("missing_distinct");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        let first_alias = root.join(".").join("first.txt");
        let mut app = App::new(first.to_str()).unwrap();

        assert!(!app.open_file_buffer(&first_alias).unwrap());
        assert!(app.open_file_buffer(&second).unwrap());
        assert_eq!(app.buffer_count(), 2);

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn alias_open_switches_synchronously_without_changing_ring_order() {
        use std::os::unix::fs::symlink;

        let root = temp_directory("alias_ring");
        let first = root.join("first.txt");
        let first_link = root.join("first-link.txt");
        let second = root.join("second.txt");
        let third = root.join("third.txt");
        fs::write(&first, "alpha").unwrap();
        fs::write(&second, "beta").unwrap();
        fs::write(&third, "gamma").unwrap();
        symlink(&first, &first_link).unwrap();
        let mut app = App::new(first.to_str()).unwrap();
        app.buffer.insert_char('!');
        app.file.dirty = true;
        app.open_file_buffer(&second).unwrap();
        app.open_file_buffer(&third).unwrap();

        assert!(app.open_file_buffer(&first_link).unwrap());
        assert_eq!(app.buffer_count(), 3);
        assert_eq!(app.buffer.to_string(), "!alpha");
        assert!(app.file.dirty);

        assert!(app.switch_buffer(BufferDirection::Next));
        assert_eq!(app.buffer.to_string(), "beta");
        assert!(app.switch_buffer(BufferDirection::Next));
        assert_eq!(app.buffer.to_string(), "gamma");

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn save_blocks_dirty_paths_that_converge_on_one_file() {
        use std::os::unix::fs::symlink;

        let root = temp_directory("save_converged");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        fs::write(&first, "alpha").unwrap();
        fs::write(&second, "beta").unwrap();
        let mut app = App::new(first.to_str()).unwrap();
        app.open_file_buffer(&second).unwrap();
        fs::remove_file(&first).unwrap();
        symlink(&second, &first).unwrap();
        app.buffer.insert_char('!');
        app.file.dirty = true;
        let mut out = Vec::new();

        save::handle_save(&mut app, &mut out).unwrap();

        assert!(app.file.dirty);
        assert_eq!(fs::read_to_string(&second).unwrap(), "beta");
        assert!(app
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("also open in another buffer"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_as_refuses_another_open_buffer_even_when_repeated() {
        let first = temp_file("save_as_first", "alpha");
        let second = temp_file("save_as_second", "beta");
        let mut app = App::new(first.to_str()).unwrap();
        app.open_file_buffer(&second).unwrap();
        app.switch_buffer(BufferDirection::Previous);
        app.buffer.insert_char('!');
        app.file.dirty = true;
        let mut out = Vec::new();

        save::handle_save_as(&mut app, &mut out, second.to_str().unwrap()).unwrap();
        save::handle_save_as(&mut app, &mut out, second.to_str().unwrap()).unwrap();

        assert_eq!(app.file.path.as_deref(), Some(first.as_path()));
        assert!(app.file.dirty);
        assert_eq!(fs::read_to_string(&second).unwrap(), "beta");
        assert!(app
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("already open in another buffer"));

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn successful_save_as_rebinds_the_active_buffer_identity() {
        let root = temp_directory("save_as_identity");
        let original = root.join("original.txt");
        let target = root.join("target.txt");
        let target_alias = root.join(".").join("target.txt");
        fs::write(&original, "alpha").unwrap();
        let mut app = App::new(original.to_str()).unwrap();
        app.buffer.insert_char('!');
        app.file.dirty = true;
        let mut out = Vec::new();

        save::handle_save_as(&mut app, &mut out, target.to_str().unwrap()).unwrap();

        assert_eq!(app.file.path.as_deref(), Some(target.as_path()));
        assert!(!app.file.dirty);
        assert!(!app.open_file_buffer(&target_alias).unwrap());
        assert_eq!(app.buffer_count(), 1);
        assert!(app.open_file_buffer(&original).unwrap());
        assert_eq!(app.buffer_count(), 2);

        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
#[path = "buffers/lifecycle_tests.rs"]
mod lifecycle_tests;
