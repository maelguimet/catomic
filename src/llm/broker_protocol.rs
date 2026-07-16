//! Purpose: this file must parse and execute the exact read-only model-to-broker protocol.
//! Owns: strict JSON command envelopes and mapping commands to budgeted broker operations.
//! Must not: infer commands from prose, write files, run tests, invoke a shell, or network.
//! Invariants: unknown/extra fields fail parsing; every successful result is broker-charged.
//! Phase: 6 (LLM Context Broker).

use std::path::Path;

use serde::Deserialize;

use super::broker::{BrokerError, ContextBroker};

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerCommand {
    ListFiles,
    ReadFile {
        path: String,
        offset: u64,
        limit: usize,
    },
    Grep {
        query: String,
    },
    ShowDiff {
        path: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    catomic_broker: BrokerCommand,
}

pub fn parse(text: &str) -> Option<BrokerCommand> {
    serde_json::from_str::<Envelope>(text)
        .ok()
        .map(|envelope| envelope.catomic_broker)
}

pub fn execute(broker: &mut ContextBroker, command: &BrokerCommand) -> Result<String, BrokerError> {
    match command {
        BrokerCommand::ListFiles => broker.list_files(),
        BrokerCommand::ReadFile {
            path,
            offset,
            limit,
        } => broker.read_file_range(Path::new(path), *offset, *limit),
        BrokerCommand::Grep { query } => broker.grep(query),
        BrokerCommand::ShowDiff { path } => broker.show_diff(Path::new(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_exact_supported_command_envelopes() {
        assert!(matches!(
            parse(r#"{"catomic_broker":{"command":"list_files"}}"#),
            Some(BrokerCommand::ListFiles)
        ));
        assert!(matches!(
            parse(
                r#"{"catomic_broker":{"command":"read_file","path":"src/lib.rs","offset":4,"limit":20}}"#
            ),
            Some(BrokerCommand::ReadFile {
                offset: 4,
                limit: 20,
                ..
            })
        ));
        for invalid in [
            "please list files",
            r#"{"command":"list_files"}"#,
            r#"{"catomic_broker":{"command":"run_tests"}}"#,
            r#"{"catomic_broker":{"command":"grep","query":"x","extra":true}}"#,
        ] {
            assert!(parse(invalid).is_none(), "accepted {invalid}");
        }
    }
}
