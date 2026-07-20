//! Purpose: measure typed parsing of a deliberately oversized command configuration.
//! Owns: the ignored 256-command, 100-parse configuration sample.
//! Must not: run by default, spawn commands, enforce machine timing, write files, or network.
//! Invariants: fixture creation is outside timing; every parse validates commands and hooks.

use crate::config::commands::{self, HookEvent};

use super::helpers::{measure_sample, print_perf_sample};

const COMMANDS: usize = 256;
const PARSES: usize = 100;

#[test]
#[ignore = "manual Phase 7 typed config parsing measurement"]
fn manual_phase7_large_config_reports_sample() {
    let mut text = String::new();
    for index in 0..COMMANDS {
        text.push_str(&format!(
            "[commands.command-{index:03}]\ncommand = \"printf {index}\"\ntimeout_secs = 10\n"
        ));
    }
    text.push_str(
        "[hooks]\non_open = [\"command-000\"]\non_save = [\"command-001\"]\n\
         before_llm = [\"command-002\"]\n",
    );

    let (config, sample) = measure_sample(
        "parse 256-command config 100x",
        Some(text.len() as u64),
        || {
            let mut parsed = commands::CommandConfig::default();
            for _ in 0..PARSES {
                parsed = commands::parse(&text).unwrap();
            }
            parsed
        },
    );
    print_perf_sample(&sample);

    assert!(config.get("command-255").is_some());
    assert_eq!(config.hooks_for(HookEvent::BeforeLlm), &["command-002"]);
}
