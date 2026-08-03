//! Purpose: construct App state from startup configuration and an optional initial path.
//! Owns: open planning and zero-work transient defaults.
//! Must not: create repository/network/process clients or start background work.
//! Invariants: startup has no repository task; watcher failure remains non-fatal.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;

#[cfg(test)]
use crate::config::big_files::BigFileConfig;
use crate::terminal as term;

use super::{
    command_prompt, completion, external_command, hooks, mobile, open, overwrite, recovery,
    replace, search, selection, startup_config::StartupConfig, surfaces, view, watch, App,
    FileState,
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
            mobile: mobile_config,
        } = config;
        let completion = completion::CompletionUiState::default();
        let mut meta = open::prepare_open_file_meta(initial_path)?;
        let buffer = open::build_open_buffer(&mut meta, initial_path, big_files.page_lines)?;
        let initial_pos = buffer.edit_history_position();
        let initial_message_role = if meta.initial_message.is_some() {
            term::render::StatusRole::Warning
        } else {
            term::render::StatusRole::Info
        };

        let mut app = App {
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
            mobile: mobile::MobileUiState::default(),
            buffer,
            file: FileState {
                path: initial_path.map(PathBuf::from),
                dirty: false,
                buffer_id: super::file_state::next_buffer_id(),
                content_generation: 0,
                saved_history_position: initial_pos,
                saved_history_pruned: false,
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
            lint: super::lint::LintState::default(),
            surfaces: surfaces::SurfaceState::default(),
            external_changes: super::external_diff::ExternalChanges::default(),
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
        if app.message.is_none() {
            app.message = highlighting_diagnostic(initial_path, theme);
            app.message_role = term::render::StatusRole::Info;
        }
        Ok(app)
    }
}

fn highlighting_diagnostic(
    initial_path: Option<&str>,
    theme: crate::config::theme::Theme,
) -> Option<String> {
    use crate::config::theme::ColorReason;
    use crate::editor::syntax::{syntax_for_path, syntax_name, SyntaxKind};

    let path = initial_path.map(std::path::Path::new)?;
    let syntax = syntax_for_path(Some(path));
    if syntax == SyntaxKind::Plain {
        return None;
    }
    let syntax_label = syntax_name(syntax);
    let plain = (syntax == SyntaxKind::Unsupported).then(|| unsupported_syntax_label(path));
    if !theme.colors_enabled {
        let reason = match theme.color_reason {
            ColorReason::NoColor => {
                "NO_COLOR is set; --color=always cannot override it".to_string()
            }
            ColorReason::ExplicitNever => "--color=never was requested".to_string(),
            ColorReason::MissingTerm => {
                "TERM is missing; use --color=always to override detection".to_string()
            }
            ColorReason::TerminalMonochrome => {
                let term = std::env::var("TERM").unwrap_or_else(|_| "<unset>".to_string());
                format!("TERM={term} is monochrome; use --color=always to override detection")
            }
            ColorReason::Automatic | ColorReason::ExplicitAlways => return None,
        };
        return Some(match plain {
            Some(label) => {
                format!("Color off: {reason}. Syntax unsupported for {label}; using plain text.")
            }
            None => format!("Color off: {reason}. Syntax: {syntax_label}."),
        });
    }
    if let Some(label) = plain {
        let diagnostic_path = path.to_string_lossy();
        return Some(format!(
            "Plain text: no syntax highlighter for {label}. Run catomic --color-diagnostics {diagnostic_path} for details."
        ));
    }
    if !syntax_theme_has_color(theme, syntax) {
        return Some(format!(
            "Syntax colors off for {syntax_label}: the selected theme defines no syntax colors."
        ));
    }
    None
}

fn unsupported_syntax_label(path: &std::path::Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .map(|extension| format!(".{extension} files"))
        .or_else(|| {
            path.file_name()
                .and_then(|filename| filename.to_str())
                .filter(|filename| !filename.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "this path".to_string())
}

fn syntax_theme_has_color(
    theme: crate::config::theme::Theme,
    syntax: crate::editor::syntax::SyntaxKind,
) -> bool {
    use crate::editor::syntax::SyntaxKind;

    let styles: &[crate::config::theme::Style] = match syntax {
        SyntaxKind::Markdown | SyntaxKind::MarkdownPreview => &[
            theme.markdown_heading,
            theme.markdown_emphasis,
            theme.markdown_code,
            theme.markdown_marker,
            theme.markdown_link,
        ],
        SyntaxKind::Diff => &[theme.diff_added, theme.diff_removed],
        SyntaxKind::Plain | SyntaxKind::Unsupported => return false,
        SyntaxKind::Rust
        | SyntaxKind::Python
        | SyntaxKind::Json
        | SyntaxKind::Toml
        | SyntaxKind::Shell => &[
            theme.syntax_keyword,
            theme.syntax_string,
            theme.syntax_comment,
            theme.syntax_number,
        ],
    };
    styles
        .iter()
        .any(|style| style.fg.is_some() || style.bg.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_startup_constructs_no_transient_surfaces() {
        let app = App::new(None).unwrap();

        assert!(app.surfaces.help.is_none());
        assert!(!super::super::external_command::is_busy(&app));
        assert!(!super::super::recovery::is_viewing(&app));
    }

    #[test]
    fn startup_explains_unsupported_and_deliberately_monochrome_syntax() {
        let mut config = StartupConfig::default();
        let unsupported = highlighting_diagnostic(Some("notes.xyz"), config.theme).unwrap();
        assert!(unsupported.contains("no syntax highlighter for .xyz files"));

        config.theme.colors_enabled = false;
        config.theme.color_reason = crate::config::theme::ColorReason::ExplicitNever;
        let monochrome = highlighting_diagnostic(Some("Cargo.toml"), config.theme).unwrap();
        assert!(monochrome.contains("--color=never") && monochrome.contains("TOML"));
    }
}
