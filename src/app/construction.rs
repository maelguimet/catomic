//! Purpose: construct App state from startup configuration and an optional initial path.
//! Owns: initial Plain capabilities, open planning, and zero-work transient defaults.
//! Must not: enter Project mode, create network/process clients, or start background work.
//! Invariants: Plain starts without Project/LLM services; watcher failure remains non-fatal.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;

#[cfg(test)]
use crate::config::big_files::BigFileConfig;
use crate::mode::{Capabilities, Mode};
use crate::terminal as term;

use super::{
    autocomplete, command_prompt, completion, external_command, hooks, inline_clanker, mobile,
    model_picker, model_session, open, overwrite, recovery, replace, search, selection,
    startup_config::StartupConfig, surfaces, view, watch, App, FileState,
};

impl App {
    #[cfg(test)]
    pub fn new(initial_path: Option<&str>) -> io::Result<Self> {
        Self::new_with_big_file_config(initial_path, BigFileConfig::default())
    }

    #[cfg(test)]
    pub(crate) fn new_with_big_file_config(
        initial_path: Option<&str>,
        big_files: BigFileConfig,
    ) -> io::Result<Self> {
        Self::new_with_config(
            initial_path,
            StartupConfig {
                big_files,
                ..StartupConfig::default()
            },
        )
    }

    pub(super) fn new_with_config(
        initial_path: Option<&str>,
        config: StartupConfig,
    ) -> io::Result<Self> {
        let StartupConfig {
            big_files,
            auto_reload,
            editor: editor_config,
            keybindings,
            commands: command_config,
            cat: cat_config,
            theme,
            view_preferences,
            autocomplete: autocomplete_config,
            mobile: mobile_config,
        } = config;
        let mode = Mode::Plain;
        let caps = Capabilities::from_mode(mode);
        let completion = caps
            .local_completion
            .then(completion::CompletionUiState::default);
        let mut meta = open::prepare_open_file_meta(initial_path)?;
        let buffer = open::build_open_buffer(&mut meta, initial_path, big_files.page_lines)?;
        let initial_pos = buffer.edit_history_position();
        let initial_message_role = if meta.initial_message.is_some() {
            term::render::StatusRole::Warning
        } else {
            term::render::StatusRole::Info
        };

        let mut app = App {
            mode,
            caps,
            project: None,
            big_files,
            auto_reload,
            editor_config,
            keybindings,
            typing_mode: overwrite::TypingMode::default(),
            command_config,
            cat_config,
            status_theme: term::render::StatusTheme::from_theme(theme),
            view_preferences,
            theme,
            autocomplete: autocomplete::AutocompleteState::new(autocomplete_config),
            mobile: mobile::MobileUiState::default(),
            buffer,
            file: FileState {
                path: initial_path.map(PathBuf::from),
                dirty: false,
                saved_history_position: initial_pos,
                disk_snapshot: meta.disk_snapshot,
                size_bytes: meta.size_bytes,
                size_tier: meta.size_tier,
                text_format: meta.text_format,
            },
            file_watcher: None,
            should_quit: false,
            message: meta.initial_message,
            message_role: initial_message_role,
            pending_quit_confirm: false,
            pending_save_conflict: None,
            pending_reload: None,
            search: search::SearchUiState::default(),
            replace: replace::ReplaceState::default(),
            command_prompt: command_prompt::CommandPromptState::default(),
            completion,
            surfaces: surfaces::SurfaceState::default(),
            pending_llm_request: None,
            llm_task: None,
            model_session: model_session::ModelSession::default(),
            model_picker: model_picker::ModelPickerState::default(),
            inline_clanker: inline_clanker::InlineClankerState::default(),
            clanker_changes: inline_clanker::ChangeHistory::default(),
            external_changes: super::external_diff::ExternalChanges::default(),
            repo_llm_state: None,
            external_command: external_command::ExternalCommandState::default(),
            hooks: hooks::HookState::default(),
            recovery: recovery::RecoveryState::default(),
            selection: selection::SelectionUiState::default(),
            clipboard: String::new(),
            cut_line_append: false,
            view: view::ViewOptions::default(),
            inactive_buffers: VecDeque::new(),
            active_buffer_index: 0,
            screen: term::screen::Screen::new(80, 24),
        };
        let mobile_enabled = mobile_config.action_bar_enabled(
            std::env::var_os("CATOMIC_MOBILE").as_deref(),
            std::env::var_os("TERMUX_VERSION").as_deref(),
        )?;
        mobile::configure(&mut app, mobile_enabled);
        watch::refresh_file_watcher(&mut app);
        recovery::initialize(&mut app);
        Ok(app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_startup_keeps_read_only_surfaces_and_explicit_tasks_absent() {
        let app = App::new(None).unwrap();

        assert!(app.project.is_none());
        assert!(app.surfaces.help.is_none());
        assert!(app.surfaces.diagnostics.is_none());
        assert!(app.surfaces.project_files.is_none());
        assert!(app.surfaces.llm_preview.is_none());
        assert!(app.pending_llm_request.is_none());
        assert!(app.llm_task.is_none());
        assert!(app.repo_llm_state.is_none());
        assert!(!model_picker::is_viewing(&app));
        assert!(!inline_clanker::is_busy(&app));
        assert!(app.autocomplete.running.is_none());
    }
}
