#!/usr/bin/env python3
"""Focused self-tests for the built-in-AI residue and ownership gate."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_no_builtin_ai as gate


VALID_MANIFEST = """\
[package]
name = "gate-fixture"
version = "0.0.0"

[package.metadata.dependencies]
ureq = "metadata, not a dependency"
# package = "ureq"

[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
tokio = { version = "1", default-features = false, features = ["rt"] }
"""

VALID_LOCK = """\
version = 4

[[package]]
name = "tokio"
version = "1.0.0"
"""


class NoBuiltInAiGateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.reset_repository()

    def reset_repository(self) -> None:
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        self.root = Path(temporary_directory.name)
        self.write("Cargo.toml", VALID_MANIFEST)
        self.write("Cargo.lock", VALID_LOCK)
        self.write(
            "src/config/config_template.toml",
            "[hooks]\non_open = []\non_save = []\n",
        )
        self.write(
            "src/app/help.rs",
            'pub const HELP: &str = "Editing, files, and recovery";\n',
        )
        self.write(
            "src/help_catalog/prompt_commands.rs",
            'pub const COMMANDS: &[&str] = &["save", "quit"];\n',
        )

    def write(self, name: str, text: str) -> None:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def violations(self) -> list[gate.Violation]:
        return gate.check_repository(self.root)

    def assert_rule(self, expected_rule: str) -> None:
        violations = self.violations()
        self.assertTrue(
            any(violation.rule == expected_rule for violation in violations),
            "\n".join(violation.render() for violation in violations),
        )

    def test_allowed_compatibility_and_non_ai_features_pass(self) -> None:
        self.write(
            "src/config/validation.rs",
            """
const RETIRED: &[&str] = &[
    "before_llm",
    "api_key_env",
    "header_envs",
    "llm_changed",
    "openai-compatible",
    "run-clanker",
    "select-model",
];
""",
        )
        self.write(
            "src/config/keybindings.rs",
            """
const RETIRED: &[&str] = &[
    "run-clanker",
    "clear-clanker-changes",
    "select-model",
    "picker-accept",
    "picker-cancel",
];
""",
        )
        self.write(
            "tests/fixtures/retired_ai_config.toml",
            """
[hooks]
before_llm = ["retired"]
[llm]
api_key_env = "RETIRED"
[[llm.backends]]
type = "openai-compatible"
header_envs = { token = "RETIRED" }
[keybindings]
run-clanker = ["f3"]
[theme.colors]
llm_changed = { fg = "red" }
""",
        )
        self.write(
            "src/app/command_prompt/tests.rs",
            'assert_eq!(run("meow rewrite"), "Unknown command: meow rewrite");\n',
        )
        self.write(
            "src/app/help/tests.rs",
            """
assert!(!help.contains("api_key_env"));
assert!(!help.contains("bigmeow INSTRUCTION"));
""",
        )
        self.write(
            "src/help_catalog/tests.rs",
            """
assert!(!catalog.contains("megameow"));
assert!(!catalog.contains("inline-meow"));
""",
        )
        self.write(
            "tests/pty_smoke.rs",
            """
assert_eq!(prompt("gitmeow inspect"), "Unknown command: gitmeow inspect");
let retired_config = "before_llm";
""",
        )
        self.write(
            "src/update/client.rs",
            """
use reqwest::Client;
fn start_runtime() {
    let _runtime = tokio::runtime::Builder::new_current_thread();
}
""",
        )
        self.write(
            "src/editor/features.rs",
            """
struct Model;
struct Modeless;
struct Diff;
struct Preview;
struct Command;
impl Command {
    fn new() -> Self { Self }
}
fn construct_domain_command() {
    let _command = Command::new();
}
use std::{fmt::Debug as process};
const COMMENT_AND_STRINGS: &str = "reqwest tokio PendingLlmRequest";
// reqwest::Client and RepoLlm are not live code here.
""",
        )
        self.write(
            "src/external/task.rs",
            """
use std::process::Command;
fn run_trusted_command(program: &str) {
    let _child = Command::new(program).spawn();
}
""",
        )

        self.assertEqual(self.violations(), [])

    def test_deleted_path_families_and_files_fail(self) -> None:
        cases = {
            "src/llm/backend.rs": "retired path family",
            "src/app/model_picker.rs": "retired file",
            "src/app/repo_llm/tests/state.rs": "nested retired path family",
        }
        for name, description in cases.items():
            with self.subTest(description):
                self.reset_repository()
                self.write(name, "pub fn resurrected() {}\n")
                self.assert_rule("deleted path")

    def test_retired_runtime_symbols_fail(self) -> None:
        for source in (
            "struct PendingLlmRequest;",
            "fn request(_: RepoLlm) {}",
            "struct OpenAiCompatClient;",
            "struct LlmConfig;",
            "struct ChatRequest;",
            "enum RepoLlmState {}",
            "struct InlineClankerState;",
            "struct ModelPickerState;",
            "const SYSTEM_PROMPT: &str = \"retired\";",
            "use crate::config::llm::Backend;",
            "mod llm;",
        ):
            with self.subTest(source):
                self.reset_repository()
                self.write("src/app/runtime.rs", source + "\n")
                violations = self.violations()
                self.assertTrue(
                    any(
                        violation.rule
                        in {"retired runtime symbol", "retired llm module"}
                        for violation in violations
                    ),
                    "\n".join(violation.render() for violation in violations),
                )

    def test_network_apis_fail_outside_updater(self) -> None:
        cases = (
            ("src/editor/network.rs", "use reqwest::Client;"),
            (
                "src/editor/network.rs",
                "fn runtime() { let _ = tokio::runtime::Builder::new_current_thread(); }",
            ),
            (
                "src/editor/network.rs",
                'fn connect() { let _ = std::net::TcpStream::connect("localhost:1"); }',
            ),
            ("src/editor/network.rs", "fn client() { let _ = hyper::Client::new(); }"),
            ("tests/network.rs", "use reqwest::Client;"),
        )
        for name, source in cases:
            with self.subTest(name=name, source=source):
                self.reset_repository()
                self.write(name, source + "\n")
                self.assert_rule("network API outside updater")

    def test_char_literals_do_not_mask_following_network_code(self) -> None:
        source = r"""
fn syntax<'a>(ch: char, borrowed: &'a str) {
    let quote = '"';
    let escaped_quote = '\'';
    let byte_quote = b'"';
    let byte_escaped_quote = b'\'';
    let byte_delete = b'\x7f';
    let delete = '\x7f';
    let cat = '\u{1f408}';
    'scan: loop {
        let _client = reqwest::Client::new();
        break 'scan;
    }
    let _ = (
        ch,
        borrowed,
        quote,
        escaped_quote,
        byte_quote,
        byte_escaped_quote,
        byte_delete,
        delete,
        cat,
    );
}
"""
        self.write("src/editor/syntax/code.rs", source)

        violations = self.violations()
        self.assertTrue(
            any(
                violation.path == "src/editor/syntax/code.rs"
                and violation.rule == "network API outside updater"
                and violation.detail.startswith("reqwest")
                for violation in violations
            ),
            "\n".join(violation.render() for violation in violations),
        )

    def test_process_apis_fail_outside_reviewed_owners(self) -> None:
        cases = (
            """
use std::process::Command;
fn ask() {
    let _ = Command::new("curl")
        .arg("https://example.test/v1/chat/completions")
        .status();
}
""",
            """
use std::process as process_api;
fn ask() {
    let _ = process_api::Command::new("curl").status();
}
""",
            """
use std::process;
fn ask() {
    let _ = process::Command::new("curl").status();
}
""",
            """
use std::process::{self as process_api};
fn ask() {
    let _ = process_api::Command::new("curl").status();
}
""",
            """
use std as standard;
fn ask() {
    let _ = standard::process::Command::new("curl").status();
}
""",
            """
use std as standard;
use standard::process::Command as Spawn;
fn ask() {
    let _ = Spawn::new("curl").status();
}
""",
            """
extern crate std as standard;
use standard::process::Command as Spawn;
fn ask() {
    let _ = Spawn::new("curl").status();
}
""",
            """
use std::{self as standard};
fn ask() {
    let _ = standard::process::Command::new("curl").status();
}
""",
            """
type Spawn = std::process::Command;
fn ask() {
    let _ = Spawn::new("curl").status();
}
""",
            """
use std::{process::{Command as Spawn}};
fn ask() {
    let _ = Spawn::new("curl").status();
}
""",
            """
use ::std::process::Command as Spawn;
fn ask() {
    let _ = Spawn::new("curl").status();
}
""",
            """
use {std::process::Command as Spawn};
fn ask() {
    let _ = Spawn::new("curl").status();
}
""",
            """
type Spawn = ::std::process::Command;
fn ask() {
    let _ = Spawn::new("curl").status();
}
""",
            """
fn ask() {
    let _ = r#std::r#process::Command::new("curl").status();
}
""",
            """
use std::{fmt, process};
fn ask() {
    let _ = process::Command::new("curl").status();
}
""",
            """
use std::process::{self};
fn ask() {
    let _ = process::Command::new("curl").status();
}
""",
            """
use std::{process::{self}};
fn ask() {
    let _ = process::Command::new("curl").status();
}
""",
            """
use std::{process::{id}, fmt::Debug as Command};
fn process_id() {
    let _ = id();
}
""",
            """
fn production_open() {
    let _ = std::process::Command::new("curl").status();
}
#[cfg(test)]
mod tests {
    fn fifo_fixture() {
        let _ = std::process::Command::new("mkfifo").status();
    }
}
""",
        )
        for index, source in enumerate(cases):
            with self.subTest(index=index, source=source):
                self.reset_repository()
                path = (
                    "src/app/open.rs"
                    if "production_open" in source
                    else "src/app/service.rs"
                )
                self.write(path, source)
                self.assert_rule("process API outside owner")

    def test_inline_test_module_process_spawn_is_not_a_production_violation(self) -> None:
        self.write(
            "src/app/open.rs",
            """
#[cfg(test)]
mod tests {
    fn fifo_fixture() {
        let _ = std::process::Command::new("mkfifo").status();
    }
}
""",
        )

        self.assertFalse(
            any(
                violation.rule == "process API outside owner"
                for violation in self.violations()
            )
        )

    def test_compatibility_tokens_fail_outside_explicit_allowances(self) -> None:
        for source, expected_rule in (
            ('const NAME: &str = "meow";', "legacy prompt name"),
            ('const ACTION: &str = "run-clanker";', "retired action name"),
            ('const FIELD: &str = "before_llm";', "retired configuration token"),
        ):
            with self.subTest(source):
                self.reset_repository()
                self.write("src/app/runtime.rs", source + "\n")
                self.assert_rule(expected_rule)

    def test_generated_surfaces_reject_ai_commands_and_settings(self) -> None:
        cases = {
            "src/config/config_template.toml": "[llm]\napi_key_env = \"TOKEN\"\n",
            "src/app/help.rs": 'pub const HELP: &str = "meow INSTRUCTION";\n',
            "src/help_catalog/prompt_commands.rs": (
                'pub const COMMANDS: &[&str] = &["select-model"];\n'
            ),
        }
        for name, text in cases.items():
            with self.subTest(name):
                self.reset_repository()
                self.write(name, text)
                violations = self.violations()
                self.assertTrue(
                    any(
                        violation.path == name
                        and violation.rule.startswith("AI term in ")
                        for violation in violations
                    ),
                    "\n".join(violation.render() for violation in violations),
                )

    def test_dependency_features_and_lock_residue_fail(self) -> None:
        self.write(
            "Cargo.toml",
            """\
[package]
name = "gate-fixture"
version = "0.0.0"

[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["macros", "rt"] }
client = { package = "ureq", version = "3" }

[dev-dependencies]
http_client = { package = "reqwest", version = "0.12" }

[target.'cfg(unix)'.dependencies]
runtime = { package = "tokio", version = "1" }

[build-dependencies."ollama-rs"]
version = "0.3"
""",
        )
        self.write(
            "Cargo.lock",
            """\
version = 4

[[package]]
name = "tokio"
version = "1.0.0"
dependencies = ["tokio-macros"]

[[package]]
name = "tokio-macros"
version = "1.0.0"
""",
        )

        violations = self.violations()
        details = {violation.detail for violation in violations}
        self.assertIn(
            "reqwest features must be exactly ['rustls-tls']",
            details,
        )
        self.assertIn("tokio default-features must be false", details)
        self.assertIn("tokio features must be exactly ['rt']", details)
        self.assertIn("tokio must not depend on tokio-macros", details)
        self.assertIn("tokio-macros must be absent", details)
        self.assertIn(
            "direct network or AI dependency is forbidden: ureq",
            details,
        )
        self.assertIn(
            "direct network or AI dependency is forbidden: ollama-rs",
            details,
        )
        self.assertIn(
            "reqwest must appear only as the exact root [dependencies].reqwest entry",
            details,
        )
        self.assertIn(
            "tokio must appear only as the exact root [dependencies].tokio entry",
            details,
        )

        self.write(
            "Cargo.lock",
            """\
version = 4

[[package]]
name = 'tokio'
version = '1.0.0'
dependencies = ['tokio-macros']

[[package]]
name = 'tokio-macros'
version = '1.0.0'
""",
        )
        single_quoted_details = {
            violation.detail for violation in self.violations()
        }
        self.assertIn("tokio must not depend on tokio-macros", single_quoted_details)
        self.assertIn("tokio-macros must be absent", single_quoted_details)

    def test_workspace_dependency_aliases_are_resolved(self) -> None:
        self.write(
            "Cargo.toml",
            """\
[package]
name = "gate-fixture"
version = "0.0.0"

[workspace]
members = []

[workspace.dependencies]
client = { package = "ureq", version = "3" }

[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
tokio = { version = "1", default-features = false, features = ["rt"] }
client = { workspace = true }
""",
        )

        details = {violation.detail for violation in self.violations()}
        self.assertIn(
            "direct network or AI dependency is forbidden: ureq",
            details,
        )


if __name__ == "__main__":
    unittest.main()
