//! Catomic — tiny entrypoint.
//!
//! The real work lives in `app` (the goblin loop) and the domain modules.
//! Keep this file boring: parse CLI, bootstrap app, run, handle top-level errors.

mod app;
mod buffer;
mod cli;
mod config;
mod editor;
mod external;
mod file;
mod help_catalog;
mod llm;
mod mode;
mod project;
mod startup_file_args;
mod terminal;
mod update;

#[cfg(test)]
mod tests;

use std::ffi::OsStr;

fn main() {
    let action = match cli::parse(std::env::args_os().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("catomic: {error}");
            std::process::exit(cli::EXIT_USAGE);
        }
    };
    let run_options = match action {
        cli::Action::Config(action) => {
            let result = match action {
                cli::ConfigAction::Path => config::user_file::print_path(),
                cli::ConfigAction::Check => config::user_file::check(),
                cli::ConfigAction::Edit => config::user_file::edit(),
            };
            if let Err(error) = result {
                eprintln!("catomic: {error}");
                std::process::exit(1);
            }
            return;
        }
        cli::Action::Help => {
            cli::print_help();
            return;
        }
        cli::Action::Version => {
            println!("catomic {}", env!("CARGO_PKG_VERSION"));
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
        cli::Action::Update(options) => {
            if let Err(error) = update::run(options) {
                eprintln!("catomic: {error}");
                std::process::exit(error.exit_code());
            }
            return;
        }
        cli::Action::Run(run_options) => run_options,
    };

    if let Err(diagnostic) = startup_file_args::check(&run_options.files, run_options.allow_missing)
    {
        eprintln!("{diagnostic}");
        std::process::exit(cli::EXIT_USAGE);
    }

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

    let result = app::run(&run_options.files);
    if let Some(signal) = terminal::termination_signal() {
        std::process::exit(128 + signal);
    }
    if let Err(e) = result {
        eprintln!("catomic: {e}");
        std::process::exit(1);
    }
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
