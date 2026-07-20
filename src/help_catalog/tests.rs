//! Purpose: verify prompt-command lookup metadata is unambiguous.
//! Owns: prompt command uniqueness, lookup, and alias regression tests.
//! Must not: duplicate configurable action metadata or construct App.
//! Invariants: every prompt command is reachable through every declared spelling.
//! Phase: issue #171 central action inventory cleanup.

use std::collections::HashSet;

use super::*;

#[test]
fn prompt_commands_and_aliases_are_unique_and_dispatchable() {
    let mut names = HashSet::new();
    for spec in PROMPT_COMMANDS {
        assert!(!spec.names.is_empty());
        for name in spec.names {
            assert!(names.insert(name), "duplicate prompt spelling: {name}");
            assert_eq!(prompt_command(name), Some(spec.command));
        }
    }
}
