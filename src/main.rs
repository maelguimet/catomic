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

fn main() {
    let action = parse_args(env::args().skip(1));
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

    if let Err(e) = app::run(&file_args) {
        eprintln!("catomic: {e}");
        std::process::exit(1);
    }
}

enum CliAction {
    Help,
    Version,
    Run(Vec<String>),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> CliAction {
    let mut files = Vec::new();
    let mut positional_only = false;
    for arg in args {
        if positional_only {
            files.push(arg);
            continue;
        }
        match arg.as_str() {
            "--" => positional_only = true,
            "-h" | "--help" => return CliAction::Help,
            "-V" | "--version" => return CliAction::Version,
            _ => files.push(arg),
        }
    }
    CliAction::Run(files)
}

fn print_help() {
    println!(
        "catomic {}\n\nUsage:\n  catomic [FILE]...\n  catomic --help\n  catomic --version\n\nInside the editor, press Ctrl+H or F1 for shortcuts.",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod cli_tests {
    use super::{parse_args, CliAction};

    #[test]
    fn parses_help_and_version_without_opening_editor() {
        assert!(matches!(
            parse_args(["--help".to_string()]),
            CliAction::Help
        ));
        assert!(matches!(
            parse_args(["-V".to_string()]),
            CliAction::Version
        ));
    }

    #[test]
    fn keeps_positional_files_and_double_dash_literals() {
        match parse_args(["a.txt".to_string(), "b.txt".to_string()]) {
            CliAction::Run(files) => assert_eq!(files, ["a.txt", "b.txt"]),
            _ => panic!("expected run action"),
        }
        match parse_args(["--".to_string(), "--help".to_string()]) {
            CliAction::Run(files) => assert_eq!(files, ["--help"]),
            _ => panic!("expected run action"),
        }
    }
}
