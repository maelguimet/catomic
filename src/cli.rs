//! Purpose: parse Catomic's small explicit command-line interface.
//! Owns: top-level actions, update flags, and usage errors.
//! Must not: inspect files, contact a network, update state, or start the editor.
//! Invariants: `update` is a subcommand only in argv[1]; `-- update` is a file.
//! Phase: safe self-update workflow.

use std::ffi::OsString;

mod help;

pub(crate) const EXIT_USAGE: i32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Config(ConfigAction),
    Help,
    Version,
    UpdateHelp,
    Update(UpdateOptions),
    ValidateConfig,
    Run(RunOptions),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RunOptions {
    pub(crate) files: Vec<String>,
    pub(crate) allow_missing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigAction {
    Path,
    Edit,
    Check,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UpdateOptions {
    pub(crate) check: bool,
    pub(crate) assume_yes: bool,
    pub(crate) backup: bool,
}

pub(crate) fn parse<I, S>(args: I) -> Result<Action, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args = utf8_args(args)?;
    if args.first().is_some_and(|arg| arg == "config") {
        return parse_config(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "update") {
        return parse_update(&args[1..]);
    }
    parse_files(args)
}

fn parse_config(args: &[String]) -> Result<Action, String> {
    let action = match args {
        [command] if command == "path" => ConfigAction::Path,
        [command] if command == "edit" => ConfigAction::Edit,
        [command] if command == "check" => ConfigAction::Check,
        [] => return Err("config requires one of: path, edit, check".to_string()),
        _ => return Err(format!("unknown config command {:?}", args.join(" "))),
    };
    Ok(Action::Config(action))
}

fn utf8_args<I, S>(args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    args.into_iter()
        .enumerate()
        .map(|(index, arg)| {
            arg.into().into_string().map_err(|_| {
                format!(
                    "argument {} is not valid UTF-8; non-UTF-8 filenames are not supported",
                    index + 1
                )
            })
        })
        .collect()
}

fn parse_update(args: &[String]) -> Result<Action, String> {
    let mut options = UpdateOptions::default();
    for arg in args {
        match help::update_option(arg) {
            Some(help::UpdateOption::Help) => return Ok(Action::UpdateHelp),
            Some(help::UpdateOption::Check) => options.check = true,
            Some(help::UpdateOption::Yes) => options.assume_yes = true,
            Some(help::UpdateOption::Backup) => options.backup = true,
            Some(help::UpdateOption::ValidateConfig) => return Ok(Action::ValidateConfig),
            None => return Err(format!("unknown update option {arg:?}")),
        }
    }
    if options.check && (options.assume_yes || options.backup) {
        return Err("--check cannot be combined with --yes or --backup".to_string());
    }
    Ok(Action::Update(options))
}

fn parse_files(args: Vec<String>) -> Result<Action, String> {
    let mut files = Vec::new();
    let mut allow_missing = false;
    let mut positional_only = false;
    for arg in args {
        if positional_only {
            files.push(arg);
            continue;
        }
        match help::main_option(&arg) {
            Some(help::MainOption::PositionalOnly) => positional_only = true,
            Some(help::MainOption::AllowMissing) => allow_missing = true,
            Some(help::MainOption::Help) => return Ok(Action::Help),
            Some(help::MainOption::Version) => return Ok(Action::Version),
            None if arg.starts_with('-') => return Err(format!("unknown option {arg:?}")),
            None => files.push(arg),
        }
    }
    Ok(Action::Run(RunOptions {
        files,
        allow_missing,
    }))
}

pub(crate) fn print_help() {
    print!("{}", help::main_help(env!("CARGO_PKG_VERSION")));
}

pub(crate) fn print_update_help() {
    print!("{}", help::update_help());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(files: &[&str], allow_missing: bool) -> Action {
        Action::Run(RunOptions {
            files: files.iter().map(|file| (*file).to_string()).collect(),
            allow_missing,
        })
    }

    #[test]
    fn parses_update_options_only_as_a_first_argument_subcommand() {
        assert_eq!(
            parse(["update", "--yes", "--backup"]).unwrap(),
            Action::Update(UpdateOptions {
                check: false,
                assume_yes: true,
                backup: true,
            })
        );
        assert_eq!(parse(["--", "update"]).unwrap(), run(&["update"], false));
        assert_eq!(
            parse(["notes", "update"]).unwrap(),
            run(&["notes", "update"], false)
        );
    }

    #[test]
    fn parses_config_discovery_commands_only_as_a_first_argument_subcommand() {
        assert_eq!(
            parse(["config", "path"]).unwrap(),
            Action::Config(ConfigAction::Path)
        );
        assert_eq!(
            parse(["config", "edit"]).unwrap(),
            Action::Config(ConfigAction::Edit)
        );
        assert_eq!(
            parse(["config", "check"]).unwrap(),
            Action::Config(ConfigAction::Check)
        );
        assert!(parse(["config"]).is_err());
        assert!(parse(["config", "wat"]).is_err());
        assert_eq!(
            parse(["--", "config"]).unwrap(),
            Action::Run(vec!["config".to_string()])
        );
    }

    #[test]
    fn rejects_conflicting_or_unknown_update_options() {
        assert!(parse(["update", "--check", "--backup"]).is_err());
        assert!(parse(["update", "--check", "--yes"]).is_err());
        assert!(parse(["update", "--wat"]).is_err());
    }

    #[test]
    fn parses_help_version_and_literal_options() {
        for spelling in ["-h", "--help"] {
            assert_eq!(parse([spelling]).unwrap(), Action::Help);
        }
        for spelling in ["-V", "--version"] {
            assert_eq!(parse([spelling]).unwrap(), Action::Version);
        }
        assert_eq!(parse(["--", "--help"]).unwrap(), run(&["--help"], false));
    }

    #[test]
    fn parses_explicit_missing_path_opt_in_without_consuming_literal_files() {
        assert_eq!(
            parse(["first", "--allow-missing", "second"]).unwrap(),
            run(&["first", "second"], true)
        );
        assert_eq!(
            parse(["--allow-missing", "--", "--help", "--allow-missing"]).unwrap(),
            run(&["--help", "--allow-missing"], true)
        );
        assert_eq!(
            parse(["--", "--allow-missing"]).unwrap(),
            run(&["--allow-missing"], false)
        );
    }

    #[test]
    fn parser_recognizes_every_cataloged_option() {
        for spec in help::MAIN_OPTIONS {
            for spelling in spec.spellings {
                assert!(parse([spelling]).is_ok(), "main parser rejected {spelling}");
            }
        }
        for spec in help::UPDATE_OPTIONS {
            for spelling in spec.spellings {
                assert!(
                    parse(["update", spelling]).is_ok(),
                    "update parser rejected {spelling}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_argument_without_panicking() {
        use std::os::unix::ffi::OsStringExt;

        let invalid_path = OsString::from_vec(b"bad-\xff-name.txt".to_vec());
        assert_eq!(
            parse([invalid_path]).unwrap_err(),
            "argument 1 is not valid UTF-8; non-UTF-8 filenames are not supported"
        );
    }
}
