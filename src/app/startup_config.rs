//! Purpose: collect inert startup settings used to construct editor buffers.
//! Owns: typed config loading, constructor grouping, and same-session cloning.
//! Must not: open buffers, render UI, construct linter services, or write files.
//! Invariants: every buffer in one session receives the same startup defaults.

use std::io;

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
    pub(super) mobile: MobileConfig,
}

impl StartupConfig {
    pub(super) fn load(color_override: crate::config::theme::ColorOverride) -> io::Result<Self> {
        let text = crate::config::user_file::read_optional()?.unwrap_or_default();
        Self::from_snapshot(
            &text,
            crate::config::view_preferences::current_path(),
            color_override,
        )
    }

    pub(super) fn without_user_config() -> io::Result<Self> {
        Self::from_snapshot("", None, crate::config::theme::ColorOverride::Auto)
    }

    fn from_snapshot(
        text: &str,
        preference_path: Option<std::path::PathBuf>,
        color_override: crate::config::theme::ColorOverride,
    ) -> io::Result<Self> {
        let document = crate::config::Document::parse(text)?;
        document.validate_unknown_keys()?;
        Ok(Self {
            big_files: crate::config::big_files::from_document(&document)?,
            auto_reload: crate::config::auto_reload::from_document(&document)?,
            editor: crate::config::editor::from_document(&document)?,
            keybindings: crate::config::keybindings::from_document(&document)?,
            commands: crate::config::commands::from_document(&document)?,
            cat: crate::config::cat::from_document(&document)?,
            theme: crate::config::theme::for_terminal(
                crate::config::theme::from_document(&document)?,
                color_override,
            ),
            view_preferences: crate::config::view_preferences::load_from_document(
                &document,
                preference_path,
            )?,
            mobile: crate::config::mobile::from_document(&document)?,
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
            crate::config::theme::ColorOverride::Auto,
        )
        .unwrap();
        assert_eq!(config.big_files.page_lines, 321);
        assert!(!config.auto_reload);
        assert_eq!(config.editor.tab_size_for_path(None), 2);
        assert!(config.view_preferences.line_numbers());
        let error = StartupConfig::from_snapshot(
            "[files]\nauto_reload = false\n[theme]\nname = \"missing\"\n",
            None,
            crate::config::theme::ColorOverride::Auto,
        )
        .err()
        .expect("one invalid recognized setting rejects the document");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let error = StartupConfig::from_snapshot(
            "[editor]\ntab_szie = 2\n",
            None,
            crate::config::theme::ColorOverride::Auto,
        )
        .err()
        .expect("unknown keys must fail startup validation");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("editor.tab_szie"));
    }

    #[test]
    fn shared_document_retains_each_sections_values_and_language_overrides() {
        let config = StartupConfig::from_snapshot(
            r#"
[big_files]
page_lines = 321
[files]
auto_reload = false
[editor]
tab_size = 2
[languages.".RS"]
tab_size = 3
linter = "check-language {file}"
[keybindings]
save = ["alt+x"]
[commands.check]
command = "check-command"
input = "buffer"
output = "preview"
timeout_secs = 12
[hooks]
on_save = ["check"]
[cat]
status_messages = false
[recovery]
enabled = true
interval_secs = 45
max_bytes = 4096
[theme]
name = "mono"
[view]
line_numbers = true
external_diff = false
[mobile]
action_bar = "always"
"#,
            None,
            crate::config::theme::ColorOverride::Auto,
        )
        .unwrap();
        assert_eq!(config.big_files.page_lines, 321);
        assert!(!config.auto_reload);
        assert_eq!(config.editor.tab_size_for_path(None), 2);
        assert_eq!(
            config
                .editor
                .tab_size_for_path(Some(std::path::Path::new("test.rs"))),
            3
        );
        assert_eq!(
            config.editor.language_linters().collect::<Vec<_>>(),
            [("rs", "check-language {file}")]
        );
        assert_eq!(
            config
                .keybindings
                .keyboard_chords(crate::config::actions::Action::Save),
            crate::config::keybindings::parse("[keybindings]\nsave = [\"alt+x\"]\n")
                .unwrap()
                .keyboard_chords(crate::config::actions::Action::Save)
        );
        let command = config.commands.get("check").unwrap();
        assert_eq!(command.command, "check-command");
        assert_eq!(command.input, crate::config::commands::CommandInput::Buffer);
        assert_eq!(command.timeout, std::time::Duration::from_secs(12));
        assert_eq!(
            config
                .commands
                .hooks_for(crate::config::commands::HookEvent::Save),
            ["check"]
        );
        assert!(!config.cat.status_messages);
        assert!(config.cat.recovery.enabled);
        assert_eq!(config.cat.recovery.interval_secs, 45);
        assert_eq!(config.cat.recovery.max_bytes, 4096);
        assert_eq!(
            config.theme,
            crate::config::theme::for_terminal(
                crate::config::theme::parse("[theme]\nname = \"mono\"\n").unwrap(),
                crate::config::theme::ColorOverride::Auto,
            )
        );
        assert!(config.view_preferences.line_numbers());
        assert!(!config.view_preferences.external_diff());
        assert_eq!(
            config.mobile.action_bar,
            crate::config::mobile::ActionBarMode::Always
        );
    }

    #[test]
    fn startup_defers_linter_mapping_validation_but_validates_language_settings() {
        let text = "[linters]\nrs = 42\n";
        assert!(StartupConfig::from_snapshot(
            text,
            None,
            crate::config::theme::ColorOverride::Auto,
        )
        .is_ok());
        let error = crate::config::validate_text(text).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("expected a string"), "{error}");
        assert!(StartupConfig::from_snapshot(
            "[languages.rs]\nlinter = \"missing-placeholder\"\n",
            None,
            crate::config::theme::ColorOverride::Auto,
        )
        .is_err());
    }

    #[test]
    fn startup_and_explicit_validation_keep_their_existing_error_precedence() {
        for (text, startup_error, validation_error) in [
            (
                "[big_files]\npage_lines = 0\n[files]\nauto_reload = \"no\"\n",
                "page_lines must be a positive integer",
                "auto_reload = \"no\"",
            ),
            (
                "[editor]\ntab_size = 0\ntab_szie = 4\n",
                "unknown configuration key editor.tab_szie",
                "unknown configuration key editor.tab_szie",
            ),
            (
                "[theme]\nname = \"missing\"\n[mobile]\naction_bar = \"invalid\"\n",
                "unknown theme",
                "action_bar",
            ),
        ] {
            let error =
                StartupConfig::from_snapshot(text, None, crate::config::theme::ColorOverride::Auto)
                    .err()
                    .unwrap();
            assert!(error.to_string().contains(startup_error), "{error}");
            let error = crate::config::validate_text(text).unwrap_err();
            assert!(error.to_string().contains(validation_error), "{error}");
        }
    }

    #[test]
    fn typed_error_keeps_the_original_toml_source_and_span() {
        #[derive(Debug, serde::Deserialize)]
        struct ConfigFile {
            #[serde(rename = "editor")]
            _editor: Editor,
        }
        #[derive(Debug, serde::Deserialize)]
        struct Editor {
            #[serde(rename = "tab_size")]
            _tab_size: usize,
        }
        let text = "# source context\n[editor]\ntab_size = \"two\"\n[commands.check]\ncommand = \"check\"\n";
        let expected = crate::config::decode::<ConfigFile>(text).unwrap_err();
        let startup_error =
            StartupConfig::from_snapshot(text, None, crate::config::theme::ColorOverride::Auto)
                .err()
                .unwrap();
        let validation_error = crate::config::validate_text(text).unwrap_err();
        assert_eq!(startup_error.to_string(), expected.to_string());
        assert_eq!(validation_error.to_string(), expected.to_string());
        assert!(startup_error.to_string().contains("line 3, column 12"));
    }

    #[test]
    fn retired_generated_autocomplete_configuration_does_not_block_startup() {
        StartupConfig::from_snapshot(
            "[autocomplete]\nenabled = false\nidle_debounce_ms = 750\n\
             minimum_prefix_length = 20\nmax_context_before = 2_048\n\
             max_context_after = 512\nmax_generated_tokens = 64\n\
             allow_remote = false\n[theme.colors]\n\
             autocomplete = { fg = \"bright-black\", dim = true }\n",
            None,
            crate::config::theme::ColorOverride::Auto,
        )
        .unwrap();
    }

    #[test]
    fn complete_retired_ai_configuration_yields_non_ai_startup_defaults() {
        let retired = StartupConfig::from_snapshot(
            include_str!("../../tests/fixtures/retired_ai_config.toml"),
            None,
            crate::config::theme::ColorOverride::Auto,
        )
        .unwrap();
        let defaults =
            StartupConfig::from_snapshot("", None, crate::config::theme::ColorOverride::Auto)
                .unwrap();

        assert_eq!(retired.big_files, defaults.big_files);
        assert_eq!(retired.auto_reload, defaults.auto_reload);
        assert_eq!(retired.editor, defaults.editor);
        assert_eq!(retired.commands, defaults.commands);
        assert_eq!(retired.cat, defaults.cat);
        assert_eq!(retired.theme, defaults.theme);
        assert_eq!(retired.view_preferences, defaults.view_preferences);
        assert_eq!(retired.mobile, defaults.mobile);
        for descriptor in crate::config::actions::REGISTRY {
            assert_eq!(
                retired.keybindings.keyboard_chords(descriptor.action),
                defaults.keybindings.keyboard_chords(descriptor.action),
                "retired AI input changed {:?}",
                descriptor.action
            );
        }
    }
}
