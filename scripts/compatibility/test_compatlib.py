#!/usr/bin/env python3
"""Purpose: prove compatibility evidence validation and immutable output behavior.
Owns: schema-invariant, focused-failure, duplicate, and overwrite regression tests.
Must not: launch Catomic, require a TTY, depend on mounts, or contact a network.
Invariants: fixtures are deterministic and every created path is temporary.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import copy
import io
import json
import tempfile
import unittest
from pathlib import Path

from build_report import (
    FILESYSTEM_BOUNDARIES,
    FILESYSTEM_REQUIRED,
    TERMINAL_REQUIRED,
    release_candidate_errors,
    validate_aggregate,
)
from compatlib import (
    EvidenceError,
    result,
    scenario,
    stage_artifact,
    validate_result,
    write_new_json,
)
from pty_driver import PtyError, PtyProcess
from run_terminal import _ask_status, _ask_yes_no
from terminal_scenarios import _core_session, _fallback_session


def fixture_scenario(status: str = "pass", issue: str | None = None, notes: str = ""):
    return scenario(
        "core-open-edit-save-quit",
        "The exact fixture is saved and Catomic exits cleanly.",
        status,
        exit_status=0,
        before_sha256="a" * 64,
        after_sha256="b" * 64,
        evidence=["fixture hash checked"],
        focused_issue=issue,
        notes=notes,
    )


def fixture_result(status: str = "pass", issue: str | None = None, notes: str = ""):
    return result(
        "run-1",
        "2026-07-18T00:00:00Z",
        "tester",
        {
            "commit": "c" * 40,
            "release": None,
            "binary_name": "catomic",
            "binary_sha256": "d" * 64,
            "binary_size": 42,
            "version_output": "catomic 0.1.0",
        },
        {
            "kind": "terminal",
            "id": "direct-pty",
            "host": {
                "os": {
                    "pretty_name": "Fixture Linux",
                    "id": "fixture",
                    "version_id": "1",
                },
                "kernel": "6.12",
                "architecture": "x86_64",
                "locale": {
                    "LC_ALL": "C.UTF-8",
                    "LC_CTYPE": "",
                    "LANG": "C.UTF-8",
                    "resolved": "C.UTF-8",
                },
                "filesystem": {
                    "probe_path": "/fixture",
                    "type": "ext4",
                    "mount_target": "/",
                    "mount_source": "/dev/fixture",
                    "mount_options": "rw",
                    "timestamp_mode": "native",
                },
            },
            "terminal": {
                "path": "direct-pty",
                "category": "pty",
                "manual": False,
                "emulator": "Linux PTY",
                "emulator_version": "Python stdlib",
                "TERM": "xterm-256color",
                "dimensions": "80x24",
                "multiplexer": "none",
                "multiplexer_version": "none",
                "ssh_path": "none",
            },
        },
        [fixture_scenario(status, issue, notes)],
    )


class EvidenceValidationTests(unittest.TestCase):
    def test_valid_pass_record(self):
        validate_result(fixture_result())

    def test_failure_requires_focused_issue(self):
        with self.assertRaisesRegex(EvidenceError, "focused GitHub issue"):
            fixture_result("fail")

        record = fixture_result(
            "fail", "https://github.com/maelguimet/catomic/issues/123"
        )
        self.assertEqual(record["overall_status"], "fail")

    def test_unsupported_requires_explanation(self):
        with self.assertRaisesRegex(EvidenceError, "must explain why"):
            fixture_result("unsupported")
        record = fixture_result("unsupported", notes="device is unavailable")
        self.assertEqual(record["overall_status"], "unsupported")

    def test_duplicate_scenario_ids_are_rejected(self):
        record = fixture_result()
        record["scenarios"].append(copy.deepcopy(record["scenarios"][0]))
        with self.assertRaisesRegex(EvidenceError, "duplicate scenario"):
            validate_result(record)

    def test_malformed_artifact_checksum_is_rejected(self):
        record = fixture_result()
        record["artifact"]["binary_sha256"] = "not-a-hash"
        with self.assertRaisesRegex(EvidenceError, "binary_sha256"):
            validate_result(record)

    def test_schema_fields_cannot_be_omitted_or_invented(self):
        missing = fixture_result()
        del missing["environment"]["host"]["filesystem"]["probe_path"]
        with self.assertRaisesRegex(EvidenceError, "missing probe_path"):
            validate_result(missing)

        invented = fixture_result()
        invented["artifact"]["build_host"] = "untrusted"
        with self.assertRaisesRegex(EvidenceError, "unexpected build_host"):
            validate_result(invented)

    def test_artifact_is_staged_before_the_source_can_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source-catomic"
            source.write_bytes(b"first candidate")
            source.chmod(0o755)
            sandbox = root / "sandbox"
            sandbox.mkdir()
            staged = stage_artifact(source, sandbox)
            source.write_bytes(b"replacement")
            self.assertEqual(staged.read_bytes(), b"first candidate")
            self.assertEqual(staged.stat().st_mode & 0o777, 0o755)

    def test_writer_refuses_to_replace_prior_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            write_new_json(path, fixture_result())
            self.assertEqual(json.loads(path.read_text())["overall_status"], "pass")
            with self.assertRaisesRegex(EvidenceError, "refusing to overwrite"):
                write_new_json(path, fixture_result())


class PtyDriverTests(unittest.TestCase):
    def test_manual_prompts_use_separate_input_and_output_streams(self):
        output = io.StringIO()
        self.assertTrue(_ask_yes_no(io.StringIO("yes\n"), output, "restored?"))
        self.assertEqual(
            _ask_status(
                io.StringIO("unsupported\nno clipboard provider\n"),
                output,
                "osc52",
                "host clipboard receives the selection",
            ),
            ("unsupported", None, "no clipboard provider"),
        )
        self.assertIn("restored? [y/n]", output.getvalue())

    def test_captures_unicode_output_and_exit_status(self):
        environment = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
        with PtyProcess(["/usr/bin/printf", "Å中🙂"], environment) as child:
            self.assertEqual(child.finish(), 0)
            self.assertEqual(child.output_text(), "Å中🙂")

    def test_wait_for_reports_missing_output_after_child_exit(self):
        environment = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
        with PtyProcess(["/usr/bin/printf", "present"], environment) as child:
            with self.assertRaisesRegex(PtyError, "timed out waiting"):
                child.wait_for(b"absent", timeout=0.2)

    def test_wait_for_occurrences_requires_fresh_behavioral_output(self):
        environment = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
        with PtyProcess(
            ["/bin/sh", "-c", "printf marker; read ignored; printf marker"],
            environment,
        ) as child:
            child.wait_for_occurrences(b"marker", 1)
            with self.assertRaisesRegex(PtyError, "2 occurrences"):
                child.wait_for_occurrences(b"marker", 2, timeout=0.1)
            child.send(b"\n")
            child.wait_for_occurrences(b"marker", 2)
            self.assertEqual(child.finish(), 0)

    def test_transcript_size_is_bounded(self):
        environment = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
        with PtyProcess(
            ["/usr/bin/printf", "12345"], environment, max_output_bytes=4
        ) as child:
            with self.assertRaisesRegex(PtyError, "exceeded 4 byte limit"):
                child.finish()


class TerminalScenarioTests(unittest.TestCase):
    def test_tmux_core_accepts_ansi_split_selection_output(self):
        rendered_marker = "SHIFTED !@# ÅΩ中🙂".encode()

        class StyledSelectionChild:
            def __init__(self, fixture):
                self.fixture = fixture
                self.output = bytearray(b"CORE_BASE")
                self.waited_for_additional_output = 0

            def send(self, data):
                if data.startswith(b"\x1b[200~"):
                    self.output.extend(rendered_marker + b"CORE_BASE")
                elif data == b"\x01":
                    self.output.extend(
                        b"\x1b[7mSHIFTED !@# \x1b[0m" + "ÅΩ中🙂".encode()
                    )
                elif data == b"\x03":
                    self.output.extend(b"selection copied")
                elif data == b"\x1a\x19\x13\x11":
                    self.fixture.write_bytes(rendered_marker + b"CORE_BASE")
                    self.output.extend(b"\x1b[?1000l\x1b[?2004l\x1b[?1049l")

            def wait_for(self, expected, timeout=5.0):
                if expected not in self.output:
                    raise PtyError(f"missing {expected!r} after {timeout} seconds")

            def wait_for_occurrences(self, expected, minimum):
                if self.output.count(expected) < minimum:
                    raise PtyError(f"missing occurrence {minimum} of {expected!r}")

            def wait_for_more_output(self, previous_length):
                if len(self.output) <= previous_length:
                    raise PtyError("missing styled selection output")
                self.waited_for_additional_output += 1

            def finish(self):
                return 0

            def close(self):
                pass

        class TmuxLauncher:
            path_id = "tmux"

            def __init__(self):
                self.child = None

            def spawn(self, _label, fixture, _environment):
                self.child = StyledSelectionChild(fixture)
                return self.child

            def cleanup(self):
                pass

        with tempfile.TemporaryDirectory() as directory:
            launcher = TmuxLauncher()
            records = _core_session(launcher, Path(directory))

        statuses = {record["id"]: record["status"] for record in records}
        self.assertEqual(statuses["core-open-edit-save-quit"], "pass")
        self.assertEqual(statuses["osc52"], "unsupported")
        self.assertEqual(launcher.child.waited_for_additional_output, 2)

    def test_fallback_prompt_close_accepts_retained_source_row(self):
        source_marker = b"FALLBACK_SOURCE_MARKER"

        class RetainedFallbackChild:
            def __init__(self):
                self.output = bytearray(source_marker)
                self.escape_count = 0
                self.waited_for_additional_output = False

            def send(self, data):
                if data == b"\x1bOP":
                    self.output.extend(b"Catomic help")
                elif data == b"\x1bOQ":
                    self.output.extend(b"Command:")
                elif data == b"\x1b":
                    self.escape_count += 1
                    if self.escape_count == 1:
                        self.output.extend(source_marker)
                    else:
                        self.output.extend(b"status row redrawn")

            def wait_for(self, expected):
                if expected not in self.output:
                    raise PtyError(f"missing {expected!r}")

            def wait_for_occurrences(self, expected, minimum):
                if self.output.count(expected) < minimum:
                    raise PtyError(f"missing occurrence {minimum} of {expected!r}")

            def wait_for_more_output(self, previous_length):
                if len(self.output) <= previous_length:
                    raise PtyError("missing retained-row redraw")
                self.waited_for_additional_output = True

            def finish(self):
                return 0

            def close(self):
                pass

        class RetainedFallbackLauncher:
            def __init__(self):
                self.child = RetainedFallbackChild()

            def spawn(self, _label, _fixture, _environment):
                return self.child

            def cleanup(self):
                pass

        with tempfile.TemporaryDirectory() as directory:
            launcher = RetainedFallbackLauncher()
            records = _fallback_session(launcher, Path(directory))

        self.assertEqual(records[0]["status"], "pass")
        self.assertTrue(launcher.child.waited_for_additional_output)


def matrix_scenario(identifier: str):
    item = fixture_scenario()
    item["id"] = identifier
    return item


def terminal_record(identifier: str, category: str, manual: bool):
    record = fixture_result()
    record["run"]["id"] = f"run-{identifier}"
    record["environment"]["id"] = identifier
    record["environment"]["terminal"]["path"] = identifier
    record["environment"]["terminal"]["category"] = category
    record["environment"]["terminal"]["manual"] = manual
    record["environment"]["terminal"]["emulator"] = identifier
    record["scenarios"] = [matrix_scenario(item) for item in sorted(TERMINAL_REQUIRED)]
    return record


def filesystem_record(identifier: str, filesystem_type: str):
    record = fixture_result()
    record["run"]["id"] = f"run-{identifier}"
    record["environment"]["kind"] = "filesystem"
    record["environment"]["id"] = identifier
    record["environment"]["host"]["filesystem"]["type"] = filesystem_type
    record["environment"]["host"]["filesystem"]["timestamp_mode"] = "frozen-mtime"
    record["scenarios"] = [
        matrix_scenario(item)
        for item in sorted(FILESYSTEM_REQUIRED | FILESYSTEM_BOUNDARIES)
    ]
    return record


class MatrixReportTests(unittest.TestCase):
    def test_aggregate_rejects_mixed_artifacts(self):
        first = fixture_result()
        second = copy.deepcopy(first)
        second["run"]["id"] = "run-2"
        second["environment"]["id"] = "tmux"
        second["artifact"]["binary_sha256"] = "e" * 64
        with self.assertRaisesRegex(EvidenceError, "same exact artifact"):
            validate_aggregate([first, second])

    def test_release_gate_requires_external_environment_evidence(self):
        errors = release_candidate_errors([terminal_record("direct-pty", "pty", False)])
        self.assertTrue(any("three materially different" in error for error in errors))
        self.assertTrue(any("two real GUI" in error for error in errors))
        self.assertTrue(any("ext4" in error for error in errors))
        self.assertTrue(any("tmpfs" in error for error in errors))

    def test_release_gate_accepts_three_paths_two_gui_and_both_filesystems(self):
        records = [
            terminal_record("direct-pty", "pty", False),
            terminal_record("vte", "gui", True),
            terminal_record("kitty", "gui", True),
            filesystem_record("ext4", "ext4"),
            filesystem_record("tmpfs", "tmpfs"),
        ]
        self.assertEqual(release_candidate_errors(records), [])

    def test_release_gate_rejects_renamed_duplicate_gui_terminals(self):
        records = [
            terminal_record("direct-pty", "pty", False),
            terminal_record("vte-one", "gui", True),
            terminal_record("vte-two", "gui", True),
            filesystem_record("ext4", "ext4"),
            filesystem_record("tmpfs", "tmpfs"),
        ]
        records[2]["environment"]["terminal"]["emulator"] = "vte-one"
        errors = release_candidate_errors(records)
        self.assertTrue(any("three materially different" in error for error in errors))
        self.assertTrue(any("two real GUI" in error for error in errors))

    def test_release_gate_rejects_linked_failures(self):
        records = [
            terminal_record("direct-pty", "pty", False),
            terminal_record("vte", "gui", True),
            terminal_record("kitty", "gui", True),
            filesystem_record("ext4", "ext4"),
            filesystem_record("tmpfs", "tmpfs"),
        ]
        records[0]["scenarios"][0]["status"] = "fail"
        records[0]["scenarios"][0]["focused_issue"] = (
            "https://github.com/maelguimet/catomic/issues/123"
        )
        records[0]["overall_status"] = "fail"
        self.assertTrue(
            any(
                "contains failed scenarios" in error
                for error in release_candidate_errors(records)
            )
        )


if __name__ == "__main__":
    unittest.main()
