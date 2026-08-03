//! Catomic — tiny entrypoint.
//!
//! The real work lives in `app` (the goblin loop) and the domain modules.
//! Keep this file boring: parse CLI, bootstrap app, run, handle top-level errors.

mod app;
mod buffer;
mod build_info;
mod cli;
mod clipboard;
mod config;
mod editor;
mod external;
mod file;
mod help_catalog;
mod process_pipe;
mod terminal;
mod update;

#[cfg(test)]
mod tests;

use std::ffi::OsStr;

enum EditorRun {
    Files(cli::RunOptions),
    Config,
}

fn main() {
    let action = match cli::parse(std::env::args_os().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("catomic: {error}");
            std::process::exit(cli::EXIT_USAGE);
        }
    };
    let editor_run = match action {
        cli::Action::Config(cli::ConfigAction::Edit) => EditorRun::Config,
        cli::Action::Config(action) => {
            let result = match action {
                cli::ConfigAction::Path => config::user_file::print_path(),
                cli::ConfigAction::Check => config::user_file::check(),
                cli::ConfigAction::RefreshKeybindings => config::user_file::refresh_keybindings(),
                cli::ConfigAction::Edit => unreachable!("edit handled as an editor run"),
            };
            if let Err(error) = result {
                eprintln!("catomic: {error}");
                std::process::exit(1);
            }
            return;
        }
        cli::Action::ConfigHelp => {
            cli::print_config_help();
            return;
        }
        cli::Action::Help => {
            cli::print_help();
            return;
        }
        cli::Action::Version => {
            println!("{}", build_info::version_line());
            return;
        }
        cli::Action::UpdateHelp => {
            cli::print_update_help();
            return;
        }
        cli::Action::ValidateConfig => {
            if let Err(error) = config::validate_all() {
                eprintln!("catomic: incompatible configuration: {error}");
                std::process::exit(update::EXIT_CONFIG);
            }
            return;
        }
        cli::Action::ColorDiagnostics(file) => {
            print_color_diagnostics(file.as_deref());
            return;
        }
        cli::Action::Update(options) => {
            if let Err(error) = update::run(options) {
                eprintln!("catomic: {error}");
                std::process::exit(error.exit_code());
            }
            return;
        }
        cli::Action::Run(run_options) => EditorRun::Files(run_options),
    };

    if let Err(error) = validate_utf8_locale(
        std::env::var_os("LC_ALL").as_deref(),
        std::env::var_os("LC_CTYPE").as_deref(),
        std::env::var_os("LANG").as_deref(),
    ) {
        eprintln!("catomic: {error}");
        std::process::exit(1);
    }

    if let Err(error) = terminal::install_process_handlers() {
        eprintln!("catomic: cannot install process signal handlers: {error}");
        std::process::exit(1);
    }

    let result = match editor_run {
        EditorRun::Files(run_options) => {
            app::run(run_options.file.as_deref(), run_options.color_override)
        }
        EditorRun::Config => app::run_config(),
    };
    if let Some(signal) = terminal::termination_signal() {
        std::process::exit(128 + signal);
    }
    if let Err(e) = result {
        eprintln!("catomic: {e}");
        std::process::exit(1);
    }
}

fn print_color_diagnostics(file: Option<&str>) {
    use config::theme::{ColorOverride, ColorReason};

    let decision = config::theme::color_decision(ColorOverride::Auto);
    let syntax = editor::syntax::syntax_for_path(file.map(std::path::Path::new));
    let term = diagnostic_environment_value("TERM");
    let colorterm = diagnostic_environment_value("COLORTERM");
    let no_color = diagnostic_environment_value("NO_COLOR");
    let reason = match decision.reason {
        ColorReason::Automatic => "terminal supports color",
        ColorReason::ExplicitAlways => "explicit override",
        ColorReason::NoColor => "NO_COLOR is set",
        ColorReason::ExplicitNever => "explicit override",
        ColorReason::MissingTerm => "TERM is missing",
        ColorReason::TerminalMonochrome => "TERM identifies a monochrome terminal",
    };
    println!("{}", build_info::version_line());
    println!("file={}", file.unwrap_or("<untitled>"));
    println!("syntax={}", editor::syntax::syntax_name(syntax));
    println!("TERM={term}");
    println!("COLORTERM={colorterm}");
    println!("NO_COLOR={no_color}");
    println!(
        "color={} ({reason})",
        if decision.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
}

fn diagnostic_environment_value(name: &str) -> String {
    std::env::var_os(name)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unset>".to_string())
}

fn validate_utf8_locale(
    lc_all: Option<&OsStr>,
    lc_ctype: Option<&OsStr>,
    lang: Option<&OsStr>,
) -> Result<(), String> {
    let selected = [("LC_ALL", lc_all), ("LC_CTYPE", lc_ctype), ("LANG", lang)]
        .into_iter()
        .find(|(_, value)| value.is_some_and(|value| !value.is_empty()));
    let Some((name, value)) = selected else {
        return Err("UTF-8 locale required; LC_ALL, LC_CTYPE, and LANG are unset".to_string());
    };
    let value = value.expect("selected locale has a non-empty value");
    let text = value
        .to_str()
        .ok_or_else(|| format!("UTF-8 locale required; {name} is not valid UTF-8"))?;
    let normalized = text.to_ascii_lowercase().replace('-', "");
    if normalized.contains("utf8") {
        Ok(())
    } else {
        Err(format!("UTF-8 locale required; {name}={text:?}"))
    }
}

#[cfg(test)]
mod cli_tests {
    use super::validate_utf8_locale;

    #[test]
    fn accepts_utf8_locale_spellings_by_precedence() {
        assert!(validate_utf8_locale(Some("C.UTF-8".as_ref()), None, None).is_ok());
        assert!(validate_utf8_locale(None, Some("en_US.utf8".as_ref()), None).is_ok());
        assert!(validate_utf8_locale(None, None, Some("fr_FR.UTF-8@euro".as_ref())).is_ok());
        assert!(validate_utf8_locale(
            Some("".as_ref()),
            Some("C.UTF-8".as_ref()),
            Some("C".as_ref())
        )
        .is_ok());
    }

    #[test]
    fn rejects_non_utf8_or_missing_locale() {
        for result in [
            validate_utf8_locale(Some("C".as_ref()), None, Some("en_US.UTF-8".as_ref())),
            validate_utf8_locale(None, Some("POSIX".as_ref()), Some("en_US.UTF-8".as_ref())),
            validate_utf8_locale(None, None, Some("C".as_ref())),
            validate_utf8_locale(None, None, None),
        ] {
            let error = result.expect_err("non-UTF-8 locale must fail closed");
            assert!(error.contains("UTF-8 locale required"));
        }
    }
}
