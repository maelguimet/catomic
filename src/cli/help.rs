//! Purpose: define and render Catomic's public command-line reference.
//! Owns: option spellings, visibility, usage forms, examples, and documentation pointers.
//! Must not: inspect paths, parse editor state, contact a network, or perform an update.
//! Invariants: CLI parsing looks up options here; internal options are explicitly hidden.
//! Phase: post-v0.1 discoverability and help-drift prevention.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MainOption {
    Help,
    Version,
    AllowMissing,
    PositionalOnly,
}

pub(super) struct MainOptionSpec {
    pub(super) option: MainOption,
    pub(super) spellings: &'static [&'static str],
    description: &'static str,
}

pub(super) const MAIN_OPTIONS: &[MainOptionSpec] = &[
    MainOptionSpec {
        option: MainOption::Help,
        spellings: &["-h", "--help"],
        description: "Show this command-line reference and exit",
    },
    MainOptionSpec {
        option: MainOption::Version,
        spellings: &["-V", "--version"],
        description: "Show the installed version and exit",
    },
    MainOptionSpec {
        option: MainOption::AllowMissing,
        spellings: &["--allow-missing"],
        description: "Allow multiple file arguments even when one or more paths are missing",
    },
    MainOptionSpec {
        option: MainOption::PositionalOnly,
        spellings: &["--"],
        description: "Treat every remaining argument as a literal file path",
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpdateOption {
    Help,
    Check,
    Yes,
    Backup,
    ValidateConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Visibility {
    Public,
    Internal,
}

pub(super) struct UpdateOptionSpec {
    pub(super) option: UpdateOption,
    pub(super) spellings: &'static [&'static str],
    description: &'static str,
    visibility: Visibility,
}

pub(super) const UPDATE_OPTIONS: &[UpdateOptionSpec] = &[
    UpdateOptionSpec {
        option: UpdateOption::Check,
        spellings: &["--check"],
        description: "Check the official source for an update without writing",
        visibility: Visibility::Public,
    },
    UpdateOptionSpec {
        option: UpdateOption::Yes,
        spellings: &["--yes"],
        description: "Apply non-interactively after all safety checks pass",
        visibility: Visibility::Public,
    },
    UpdateOptionSpec {
        option: UpdateOption::Backup,
        spellings: &["--backup"],
        description: "Back up user-owned configuration and state before applying",
        visibility: Visibility::Public,
    },
    UpdateOptionSpec {
        option: UpdateOption::Help,
        spellings: &["-h", "--help"],
        description: "Show this detailed updater reference and exit",
        visibility: Visibility::Public,
    },
    UpdateOptionSpec {
        option: UpdateOption::ValidateConfig,
        spellings: &["--validate-config"],
        description: "Validate configuration for the updater's staged candidate",
        visibility: Visibility::Internal,
    },
];

pub(super) fn main_option(spelling: &str) -> Option<MainOption> {
    MAIN_OPTIONS
        .iter()
        .find(|spec| spec.spellings.contains(&spelling))
        .map(|spec| spec.option)
}

pub(super) fn update_option(spelling: &str) -> Option<UpdateOption> {
    UPDATE_OPTIONS
        .iter()
        .find(|spec| spec.spellings.contains(&spelling))
        .map(|spec| spec.option)
}

pub(super) fn main_help(version: &str) -> String {
    format!(
        concat!(
            "catomic {}\n\n",
            "Catomic is a Linux-first, modeless terminal text editor.\n\n",
            "Usage:\n",
            "  catomic [OPTION]... [--] [FILE]...\n",
            "  catomic update [--yes] [--backup]\n",
            "  catomic update --check\n",
            "  catomic (-h | --help)\n",
            "  catomic (-V | --version)\n",
            "  catomic update (-h | --help)\n\n",
            "Files:\n",
            "  With no FILE, Catomic opens one untitled empty buffer. One FILE opens one\n",
            "  buffer; multiple FILE arguments open multiple buffers in argument order.\n",
            "  One missing path opens as an empty named buffer and is created only when\n",
            "  saved. With multiple FILE arguments, every path must exist unless explicit\n",
            "  --allow-missing is present. Paths and contents must be UTF-8.\n\n",
            "Options:\n",
            "{}\n",
            "Subcommands:\n",
            "  update  Check or apply a safe, install-aware Catomic update.\n",
            "          Run `catomic update --help` for updater options, network/write\n",
            "          behavior, backup, and rollback details.\n\n",
            "Examples:\n",
            "  catomic\n",
            "  catomic notes.md todo.txt\n",
            "  catomic --allow-missing draft.md notes.md\n",
            "  catomic \"meeting notes.md\"  Shell-quote a path containing spaces\n",
            "  catomic -- -draft.md         Open an option-like filename\n",
            "  catomic -- update            Open a first file literally named update\n",
            "  catomic update --check       Check without changing local state\n\n",
            "Configuration:\n",
            "  $XDG_CONFIG_HOME/catomic/config.toml (when XDG_CONFIG_HOME is absolute),\n",
            "  otherwise ~/.config/catomic/config.toml. No configuration is required.\n\n",
            "Learn more:\n",
            "  https://github.com/maelguimet/catomic/blob/master/docs/user-guide.md\n",
            "  Inside the editor, press Ctrl+H or F1 for the default-key and command\n",
            "  quick reference.\n",
        ),
        version,
        render_main_options()
    )
}

pub(super) fn update_help() -> String {
    format!(
        concat!(
            "catomic update\n\n",
            "Check or apply an update for a supported official Catomic installation.\n\n",
            "Usage:\n",
            "  catomic update\n",
            "  catomic update [--yes] [--backup]\n",
            "  catomic update --check\n",
            "  catomic update (-h | --help)\n\n",
            "Options:\n",
            "{}\n",
            "The default command prints its trusted source and asks before network or\n",
            "install actions. --check may contact the official source but does not write.\n",
            "--check cannot be combined with --yes or --backup. Configuration is never\n",
            "rewritten; successful installs retain a rollback binary and print its path.\n\n",
            "Full backup, install-method, rollback, and exit-code reference:\n",
            "  https://github.com/maelguimet/catomic/blob/master/docs/user-guide.md#updating-backup-and-rollback\n",
        ),
        render_update_options()
    )
}

fn render_main_options() -> String {
    let mut text = String::new();
    for spec in MAIN_OPTIONS {
        push_option(&mut text, spec.spellings, spec.description);
    }
    text
}

fn render_update_options() -> String {
    let mut text = String::new();
    for spec in UPDATE_OPTIONS
        .iter()
        .filter(|spec| spec.visibility == Visibility::Public)
    {
        push_option(&mut text, spec.spellings, spec.description);
    }
    text
}

fn push_option(text: &mut String, spellings: &[&str], description: &str) {
    text.push_str("  ");
    text.push_str(&spellings.join(", "));
    text.push('\n');
    text.push_str("      ");
    text.push_str(description);
    text.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_public_option_is_rendered_and_internal_option_is_hidden() {
        let main = main_help("test-version");
        for spec in MAIN_OPTIONS {
            for spelling in spec.spellings {
                assert!(main.contains(spelling), "main help is missing {spelling}");
            }
        }

        let update = update_help();
        for spec in UPDATE_OPTIONS {
            for spelling in spec.spellings {
                assert_eq!(
                    update.contains(spelling),
                    spec.visibility == Visibility::Public,
                    "wrong visibility for {spelling}"
                );
            }
        }
    }

    #[test]
    fn main_help_covers_file_semantics_quoting_examples_and_pointers() {
        let text = main_help("test-version");
        for required in [
            "no FILE",
            "multiple FILE arguments open multiple buffers in argument order",
            "One missing path opens as an empty named buffer",
            "created only when",
            "every path must exist unless explicit",
            "--allow-missing",
            "catomic -- -draft.md",
            "catomic \"meeting notes.md\"",
            "catomic update --help",
            "$XDG_CONFIG_HOME/catomic/config.toml",
            "docs/user-guide.md",
        ] {
            assert!(text.contains(required), "main help is missing {required:?}");
        }
    }
}
