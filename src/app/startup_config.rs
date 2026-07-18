//! Purpose: collect Plain-safe startup settings used to construct editor buffers.
//! Owns: typed config loading, constructor grouping, and same-session cloning.
//! Must not: open buffers, render UI, construct Project/LLM services, or write files.
//! Invariants: every buffer in one session receives the same startup defaults.
//! Phase: post-v0.1 configuration plumbing.

use std::io;

use crate::config::big_files::BigFileConfig;
use crate::config::cat::CatConfig;
use crate::config::commands::CommandConfig;
use crate::config::editor::EditorConfig;
use crate::config::keybindings::KeyBindings;
use crate::config::view_preferences::ViewPreferences;

#[derive(Clone)]
pub(super) struct StartupConfig {
    pub(super) big_files: BigFileConfig,
    pub(super) auto_reload: bool,
    pub(super) editor: EditorConfig,
    pub(super) keybindings: KeyBindings,
    pub(super) commands: CommandConfig,
    pub(super) cat: CatConfig,
    pub(super) view_preferences: ViewPreferences,
}

impl StartupConfig {
    pub(super) fn load() -> io::Result<Self> {
        Ok(Self {
            big_files: crate::config::big_files::load()?,
            auto_reload: crate::config::auto_reload::load()?,
            editor: crate::config::editor::load()?,
            keybindings: crate::config::keybindings::load()?,
            commands: crate::config::commands::load()?,
            cat: crate::config::cat::load()?,
            view_preferences: crate::config::view_preferences::load()?,
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
            view_preferences: app.view_preferences.clone(),
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
            view_preferences: ViewPreferences::default(),
        }
    }
}
