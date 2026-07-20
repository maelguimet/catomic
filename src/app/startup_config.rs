//! Purpose: collect Plain-safe startup settings used to construct editor buffers.
//! Owns: typed config loading, constructor grouping, and same-session cloning.
//! Must not: open buffers, render UI, construct Project/LLM services, or write files.
//! Invariants: every buffer in one session receives the same startup defaults.

use std::io;

use crate::config::autocomplete::AutocompleteConfig;
use crate::config::big_files::BigFileConfig;
use crate::config::cat::CatConfig;
use crate::config::commands::CommandConfig;
use crate::config::editor::EditorConfig;
use crate::config::keybindings::KeyBindings;
use crate::config::mobile::MobileConfig;
use crate::config::theme::Theme;
use crate::config::view_preferences::ViewPreferences;

#[derive(Clone)]
pub(super) struct StartupConfig {
    pub(super) big_files: BigFileConfig,
    pub(super) auto_reload: bool,
    pub(super) editor: EditorConfig,
    pub(super) keybindings: KeyBindings,
    pub(super) commands: CommandConfig,
    pub(super) cat: CatConfig,
    pub(super) theme: Theme,
    pub(super) view_preferences: ViewPreferences,
    pub(super) autocomplete: AutocompleteConfig,
    pub(super) mobile: MobileConfig,
}

impl StartupConfig {
    pub(super) fn load() -> io::Result<Self> {
        let text = crate::config::user_file::read_optional()?.unwrap_or_default();
        Self::from_snapshot(&text, crate::config::view_preferences::current_path())
    }

    pub(super) fn without_user_config() -> io::Result<Self> {
        Self::from_snapshot("", None)
    }

    fn from_snapshot(text: &str, preference_path: Option<std::path::PathBuf>) -> io::Result<Self> {
        Ok(Self {
            big_files: crate::config::big_files::parse(text)?,
            auto_reload: crate::config::auto_reload::parse(text)?,
            editor: crate::config::editor::parse(text)?,
            keybindings: crate::config::keybindings::parse(text)?,
            commands: crate::config::commands::parse(text)?,
            cat: crate::config::cat::parse(text)?,
            theme: crate::config::theme::for_terminal(crate::config::theme::parse(text)?),
            view_preferences: crate::config::view_preferences::load_with_config(
                text,
                preference_path,
            )?,
            autocomplete: crate::config::autocomplete::parse(text)?,
            mobile: crate::config::mobile::parse(text)?,
        })
    }

    pub(super) fn for_new_buffer(app: &super::App) -> Self {
        Self {
            big_files: app.big_files,
            auto_reload: app.auto_reload,
            editor: app.editor_config.clone(),
            keybindings: app.keybindings.clone(),
            commands: app.command_config.clone(),
            cat: app.cat_config,
            theme: app.theme,
            view_preferences: app.view_preferences.clone(),
            autocomplete: app.autocomplete.config.clone(),
            mobile: MobileConfig {
                action_bar: crate::config::mobile::ActionBarMode::from_enabled(
                    super::mobile::is_enabled(app),
                ),
            },
        }
    }
}

#[cfg(test)]
impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            big_files: BigFileConfig::default(),
            auto_reload: true,
            editor: EditorConfig::default(),
            keybindings: KeyBindings::default(),
            commands: CommandConfig::default(),
            cat: CatConfig::default(),
            theme: Theme::default(),
            view_preferences: ViewPreferences::default(),
            autocomplete: AutocompleteConfig::default(),
            mobile: MobileConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_snapshot_populates_all_startup_sections_or_fails_as_a_unit() {
        let config = StartupConfig::from_snapshot(
            "[big_files]\npage_lines = 321\n[files]\nauto_reload = false\n\
             [editor]\ntab_size = 2\n[view]\nline_numbers = true\n\
             [theme]\nname = \"high-contrast\"\n",
            None,
        )
        .unwrap();
        assert_eq!(config.big_files.page_lines, 321);
        assert!(!config.auto_reload);
        assert_eq!(config.editor.tab_size_for_path(None), 2);
        assert!(config.view_preferences.line_numbers());
        assert!(!config.autocomplete.enabled);

        let error = StartupConfig::from_snapshot(
            "[files]\nauto_reload = false\n[theme]\nname = \"missing\"\n",
            None,
        )
        .err()
        .expect("one invalid recognized setting rejects the document");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
