//! Purpose: parse Catomic's small explicit command-line interface.
//! Owns: top-level actions, config/update flags, and usage errors.
//! Must not: inspect files, contact a network, update state, or start the editor.
//! Invariants: reserved commands are recognized only in argv[1]; file words
//! join one path.
//! Phase: safe self-update workflow.

use std::ffi::OsString;

mod help;

pub(crate) const EXIT_USAGE: i32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Config(ConfigAction),
    ConfigHelp,
    Help,
    Version,
    UpdateHelp,
    Update(UpdateOptions),
    ValidateConfig,
    Run(RunOptions),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RunOptions {
    pub(crate) file: Option<String>,
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
    parse_file(args)
}

fn parse_config(args: &[String]) -> Result<Action, String> {
    let action = match args {
        [] => ConfigAction::Edit,
        [command] if command == "path" => ConfigAction::Path,
        [command] if command == "edit" => ConfigAction::Edit,
        [command] if command == "check" => ConfigAction::Check,
        [option] if help::config_option(option) == Some(help::ConfigOption::Help) => {
            return Ok(Action::ConfigHelp);
        }
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
    if args
        .first()
        .is_some_and(|arg| matches!(help::update_option(arg), Some(help::UpdateOption::Help)))
    {
        return match args {
            [_] => Ok(Action::UpdateHelp),
            _ => Err(format!(
                "{} does not take arguments",
                args.first().expect("help argument exists")
            )),
        };
    }
    let mut options = UpdateOptions::default();
    for arg in args {
        match help::update_option(arg) {
            Some(help::UpdateOption::Help) => {
                return Err(format!("{arg} must immediately follow `catomic update`"));
            }
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

fn parse_file(args: Vec<String>) -> Result<Action, String> {
    let Some(first) = args.first() else {
        return Ok(Action::Run(RunOptions::default()));
    };
    match help::main_option(first) {
        Some(help::MainOption::Help) if args.len() == 1 => Ok(Action::Help),
        Some(help::MainOption::Version) if args.len() == 1 => Ok(Action::Version),
        Some(help::MainOption::Help | help::MainOption::Version) => Err(format!(
            "{first} does not take arguments; prefix an option-like filename with `./`"
        )),
        None if first.starts_with('-') => Err(format!("unknown option {first:?}")),
        None => Ok(run_file(&args)),
    }
}

fn run_file(words: &[String]) -> Action {
    Action::Run(RunOptions {
        file: (!words.is_empty()).then(|| words.join(" ")),
    })
}

pub(crate) fn print_help() {
    print!("{}", help::main_help(env!("CARGO_PKG_VERSION")));
}

pub(crate) fn print_config_help() {
    print!("{}", help::config_help());
}

pub(crate) fn print_update_help() {
    print!("{}", help::update_help());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(file: Option<&str>) -> Action {
        Action::Run(RunOptions {
            file: file.map(str::to_string),
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
        assert_eq!(
            parse(["notes", "update"]).unwrap(),
            run(Some("notes update"))
        );
        assert!(parse(["--", "update"]).is_err());
    }

    #[test]
    fn parses_config_commands_only_as_a_first_argument_subcommand() {
        assert_eq!(
            parse(["config"]).unwrap(),
            Action::Config(ConfigAction::Edit)
        );
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
        for spelling in ["-h", "--help"] {
            assert_eq!(parse(["config", spelling]).unwrap(), Action::ConfigHelp);
        }
        assert!(parse(["config", "wat"]).is_err());
        assert!(parse(["--", "config"]).is_err());
    }

    #[test]
    fn command_name_files_use_an_explicit_relative_path() {
        assert_eq!(parse(["./update"]).unwrap(), run(Some("./update")));
        assert_eq!(parse(["./config"]).unwrap(), run(Some("./config")));
        assert_eq!(parse(["./-draft.md"]).unwrap(), run(Some("./-draft.md")));
    }

    #[test]
    fn rejects_conflicting_or_unknown_update_options() {
        assert!(parse(["update", "--check", "--backup"]).is_err());
        assert!(parse(["update", "--check", "--yes"]).is_err());
        assert!(parse(["update", "--wat"]).is_err());
    }

    #[test]
    fn parses_help_and_version_and_rejects_positional_only_syntax() {
        for spelling in ["-h", "--help"] {
            assert_eq!(parse([spelling]).unwrap(), Action::Help);
        }
        for spelling in ["-V", "--version"] {
            assert_eq!(parse([spelling]).unwrap(), Action::Version);
        }
        assert!(parse(["--"]).is_err());
        assert!(parse(["--", "--help"]).is_err());
    }

    #[test]
    fn joins_non_command_words_into_one_file_path() {
        assert_eq!(
            parse(["hello", "world.md"]).unwrap(),
            run(Some("hello world.md"))
        );
        assert_eq!(
            parse(["hello", "--help"]).unwrap(),
            run(Some("hello --help"))
        );
        assert_eq!(parse(Vec::<String>::new()).unwrap(), run(None));
    }

    #[test]
    fn commands_that_do_not_accept_arguments_fail_loudly() {
        assert!(parse(["--help", "notes.md"])
            .unwrap_err()
            .contains("does not take arguments"));
        assert!(parse(["--version", "notes.md"])
            .unwrap_err()
            .contains("does not take arguments"));
        assert!(parse(["update", "--help", "notes.md"])
            .unwrap_err()
            .contains("does not take arguments"));
        assert!(parse(["update", "world.md"])
            .unwrap_err()
            .contains("unknown update option"));
        assert!(parse(["config", "path", "notes.md"]).is_err());
    }

    #[test]
    fn parser_recognizes_every_cataloged_option() {
        for spec in help::MAIN_OPTIONS {
            for spelling in spec.spellings {
                assert!(parse([spelling]).is_ok(), "main parser rejected {spelling}");
            }
        }
        for spec in help::CONFIG_OPTIONS {
            for spelling in spec.spellings {
                assert!(
                    parse(["config", spelling]).is_ok(),
                    "config parser rejected {spelling}"
                );
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
