//! Catomic — tiny entrypoint.
//!
//! The real work lives in `app` (the goblin loop) and the domain modules.
//! Keep this file boring: parse CLI, bootstrap app, run, handle top-level errors.

mod app;
mod buffer;
mod config;
mod editor;
mod external;
mod file;
mod llm;
mod mode;
mod project;
mod terminal;

#[cfg(test)]
mod tests;

use std::env;
use std::ffi::{OsStr, OsString};

fn main() {
    let action = match parse_args(env::args_os().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("catomic: {error}");
            std::process::exit(1);
        }
    };
    let file_args = match action {
        CliAction::Help => {
            print_help();
            return;
        }
        CliAction::Version => {
            println!("catomic {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        CliAction::Run(file_args) => file_args,
    };

    if let Err(error) = validate_utf8_locale(
        env::var_os("LC_ALL").as_deref(),
        env::var_os("LC_CTYPE").as_deref(),
        env::var_os("LANG").as_deref(),
    ) {
        eprintln!("catomic: {error}");
        std::process::exit(1);
    }

    if let Err(error) = terminal::install_process_handlers() {
        eprintln!("catomic: cannot install process signal handlers: {error}");
        std::process::exit(1);
    }

    let result = app::run(&file_args);
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

enum CliAction {
    Help,
    Version,
    Run(Vec<String>),
}

fn parse_args<I, S>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut files = Vec::new();
    let mut positional_only = false;
    for (index, arg) in args.into_iter().enumerate() {
        let arg = arg.into().into_string().map_err(|_| {
            format!(
                "argument {} is not valid UTF-8; non-UTF-8 filenames are not supported",
                index + 1
            )
        })?;
        if positional_only {
            files.push(arg);
            continue;
        }
        match arg.as_str() {
            "--" => positional_only = true,
            "-h" | "--help" => return Ok(CliAction::Help),
            "-V" | "--version" => return Ok(CliAction::Version),
            _ => files.push(arg),
        }
    }
    Ok(CliAction::Run(files))
}

fn print_help() {
    println!(
        "catomic {}\n\nUsage:\n  catomic [FILE]...\n  catomic --help\n  catomic --version\n\nInside the editor, press Ctrl+H or F1 for shortcuts.",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod cli_tests {
    use super::{parse_args, validate_utf8_locale, CliAction};

    #[test]
    fn parses_help_and_version_without_opening_editor() {
        assert!(matches!(
            parse_args(["--help".to_string()]).unwrap(),
            CliAction::Help
        ));
        assert!(matches!(
            parse_args(["-V".to_string()]).unwrap(),
            CliAction::Version
        ));
    }

    #[test]
    fn keeps_positional_files_and_double_dash_literals() {
        match parse_args(["a.txt".to_string(), "b.txt".to_string()]).unwrap() {
            CliAction::Run(files) => assert_eq!(files, ["a.txt", "b.txt"]),
            _ => panic!("expected run action"),
        }
        match parse_args(["--".to_string(), "--help".to_string()]).unwrap() {
            CliAction::Run(files) => assert_eq!(files, ["--help"]),
            _ => panic!("expected run action"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_argument_without_panicking() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let invalid_path = OsString::from_vec(b"bad-\xff-name.txt".to_vec());
        let error = parse_args([invalid_path])
            .err()
            .expect("non-UTF-8 argument must be rejected");

        assert_eq!(
            error,
            "argument 1 is not valid UTF-8; non-UTF-8 filenames are not supported"
        );
    }

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
