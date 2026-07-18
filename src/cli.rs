//! Purpose: parse Catomic's small explicit command-line interface.
//! Owns: top-level actions, update flags, usage errors, and help text.
//! Must not: inspect files, contact a network, update state, or start the editor.
//! Invariants: `update` is a subcommand only in argv[1]; `-- update` is a file.
//! Phase: safe self-update workflow.

use std::ffi::OsString;

pub(crate) const EXIT_USAGE: i32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
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
    if args.first().is_some_and(|arg| arg == "update") {
        return parse_update(&args[1..]);
    }
    parse_files(args)
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
        match arg.as_str() {
            "-h" | "--help" => return Ok(Action::UpdateHelp),
            "--check" => options.check = true,
            "--yes" => options.assume_yes = true,
            "--backup" => options.backup = true,
            "--validate-config" => return Ok(Action::ValidateConfig),
            _ => return Err(format!("unknown update option {arg:?}")),
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
        match arg.as_str() {
            "--" => positional_only = true,
            "--allow-missing" => allow_missing = true,
            "-h" | "--help" => return Ok(Action::Help),
            "-V" | "--version" => return Ok(Action::Version),
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg:?}")),
            _ => files.push(arg),
        }
    }
    Ok(Action::Run(RunOptions {
        files,
        allow_missing,
    }))
}

pub(crate) fn print_help() {
    println!(
        "catomic {}\n\nUsage:\n  catomic [--allow-missing] [FILE]...\n  catomic update [--check] [--yes] [--backup]\n  catomic --help\n  catomic --version\n\nOptions:\n  --allow-missing\n            Explicitly allow several file arguments when any do not exist\n\nUse `catomic -- update` to open a file literally named `update`.\nInside the editor, press Ctrl+H or F1 for shortcuts.",
        env!("CARGO_PKG_VERSION")
    );
}

pub(crate) fn print_update_help() {
    println!(
        "catomic update\n\nUsage:\n  catomic update\n  catomic update --check\n  catomic update --yes\n  catomic update --backup\n\nOptions:\n  --check   Report update availability without writing anything\n  --yes     Apply non-interactively\n  --backup  Back up user-owned configuration and state before applying\n  -h, --help\n            Show this help"
    );
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
    fn rejects_conflicting_or_unknown_update_options() {
        assert!(parse(["update", "--check", "--backup"]).is_err());
        assert!(parse(["update", "--check", "--yes"]).is_err());
        assert!(parse(["update", "--wat"]).is_err());
    }

    #[test]
    fn parses_help_version_and_literal_options() {
        assert_eq!(parse(["--help"]).unwrap(), Action::Help);
        assert_eq!(parse(["-V"]).unwrap(), Action::Version);
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
