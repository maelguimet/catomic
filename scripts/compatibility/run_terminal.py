#!/usr/bin/env python3
"""Purpose: record automated or operator-attested terminal compatibility evidence.
Owns: command-line contract, isolated fixtures, manual checklist, and result assembly.
Must not: infer GUI behavior from a PTY, overwrite evidence, publish, or contact a network.
Invariants: every result binds one binary checksum/commit and names the exact terminal path.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from compatlib import (
    EvidenceError,
    artifact,
    host_environment,
    result,
    scenario,
    sha256_file,
    stage_artifact,
    utc_now,
    write_new_json,
)
from pty_driver import isolated_environment
from terminal_contract import TERMINAL_SCENARIOS
from terminal_path import terminal_details
from terminal_scenarios import run_automated_terminal


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="mode", required=True)
    automated = subparsers.add_parser("automated", help="drive direct PTY or tmux")
    _common_arguments(automated)
    automated.add_argument("--path", choices=("direct-pty", "tmux"), required=True)
    automated.add_argument("--failure-issue")
    automated.add_argument("--work-root", type=Path)
    automated.add_argument("--keep-sandbox", action="store_true")

    manual = subparsers.add_parser(
        "manual", help="run an operator checklist in this terminal"
    )
    _common_arguments(manual)
    manual.add_argument("--path-id", required=True)
    manual.add_argument(
        "--category", choices=("pty", "multiplexer", "remote", "gui"), required=True
    )
    manual.add_argument("--terminal", required=True)
    manual.add_argument("--terminal-version", required=True)
    manual.add_argument("--multiplexer", default="none")
    manual.add_argument("--multiplexer-version", default="none")
    manual.add_argument("--ssh-path", default="none")
    manual.add_argument("--work-root", type=Path)
    manual.add_argument("--keep-sandbox", action="store_true")
    return parser.parse_args()


def _common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--release")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--operator", required=True)
    parser.add_argument("--run-id", default=f"terminal-{uuid.uuid4()}")


def main() -> int:
    args = parse_args()
    started = utc_now()
    if args.work_root is not None and (
        args.work_root.is_symlink() or not args.work_root.is_dir()
    ):
        raise EvidenceError("work root must be a non-symlink directory")
    work_root = args.work_root.resolve(strict=True) if args.work_root else None
    sandbox = Path(tempfile.mkdtemp(prefix="catomic-terminal-", dir=work_root))
    try:
        candidate = stage_artifact(args.binary, sandbox)
        artifact_record = artifact(candidate, args.commit, args.release)
        if args.mode == "automated":
            scenarios = run_automated_terminal(
                candidate, args.path, sandbox, args.failure_issue
            )
            terminal = terminal_details(args.path)
            environment_id = args.path
        else:
            terminal, scenarios = run_manual(args, sandbox, candidate)
            environment_id = args.path_id
        environment = {
            "kind": "terminal",
            "id": environment_id,
            "host": host_environment(sandbox, "native"),
            "terminal": terminal,
        }
        record = result(
            args.run_id,
            started,
            args.operator,
            artifact_record,
            environment,
            scenarios,
        )
        write_new_json(args.output, record)
        print(f"compatibility result: {args.output}")
        print(f"artifact sha256: {artifact_record['binary_sha256']}")
        print(f"overall status: {record['overall_status']}")
        return 0 if record["overall_status"] != "fail" else 1
    finally:
        if args.keep_sandbox:
            print(f"sandbox retained: {sandbox}", file=sys.stderr)
        else:
            shutil.rmtree(sandbox, ignore_errors=True)


def run_manual(args: argparse.Namespace, sandbox: Path, candidate: Path):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise EvidenceError("manual terminal evidence requires an interactive TTY")
    fixture = sandbox / "manual-terminal.txt"
    fixture.write_text("Catomic compatibility fixture\nsecond line\n", encoding="utf-8")
    before = sha256_file(fixture)
    tty = Path("/dev/tty").open("r+", encoding="utf-8", buffering=1)
    try:
        _print_manual_instructions(tty)
        tty.write("Press Enter to launch the exact binary... ")
        tty.readline()
        stty_before = _stty_state(tty)
        dimensions_before = shutil.get_terminal_size()
        environment = isolated_environment(
            sandbox / "manual-env", os.environ.get("TERM", "unknown")
        )
        completed = subprocess.run(
            [str(candidate), str(fixture)],
            env=environment,
            check=False,
        )
        stty_after = _stty_state(tty)
        after = sha256_file(fixture)
        visual_restore = _ask_yes_no(
            tty, "Did the normal screen, cursor, echo, and input mode return?"
        )
        restore = {
            "restored": stty_before == stty_after and visual_restore,
            "stty_before": stty_before,
            "stty_after": stty_after,
            "teardown_sequences": None,
        }
        records = []
        for identifier in TERMINAL_SCENARIOS:
            expected = _manual_expected(identifier)
            status, issue, notes = _ask_status(tty, identifier, expected)
            records.append(
                scenario(
                    identifier,
                    expected,
                    status,
                    exit_status=completed.returncode,
                    before_sha256=before,
                    after_sha256=after,
                    evidence=["operator attestation from the named terminal path"],
                    restoration=restore
                    if identifier in {"signals", "terminal-restoration"}
                    else None,
                    focused_issue=issue,
                    notes=notes,
                )
            )
        if not restore["restored"] and all(
            item["status"] != "fail" for item in records
        ):
            raise EvidenceError(
                "terminal state differed but no focused failure was recorded"
            )
        terminal = {
            "path": args.path_id,
            "category": args.category,
            "manual": True,
            "emulator": args.terminal,
            "emulator_version": args.terminal_version,
            "TERM": os.environ.get("TERM", "unknown"),
            "dimensions": f"{dimensions_before.columns}x{dimensions_before.lines}",
            "multiplexer": args.multiplexer,
            "multiplexer_version": args.multiplexer_version,
            "ssh_path": args.ssh_path,
        }
        return terminal, records
    finally:
        tty.close()


def _print_manual_instructions(tty) -> None:
    tty.write(
        "\nCatomic manual terminal checklist\n"
        "Use only the generated non-sensitive fixture. Exercise open/edit/save/quit; type\n"
        "uppercase and shifted punctuation plus ÅΩ中🙂; use F1 and F2; click and type at\n"
        "a known position; paste with the terminal; copy and verify the host clipboard;\n"
        "resize narrower and wider; and verify clean/signal restoration. Record unsupported\n"
        "when this path cannot expose a capability. Do not mark inferred behavior as pass.\n\n"
    )


def _stty_state(tty) -> str:
    completed = subprocess.run(
        ["stty", "-g"], stdin=tty, text=True, capture_output=True, check=True, timeout=5
    )
    return completed.stdout.strip()


def _ask_yes_no(tty, prompt: str) -> bool:
    while True:
        tty.write(f"{prompt} [y/n] ")
        answer = tty.readline().strip().lower()
        if answer in {"y", "yes"}:
            return True
        if answer in {"n", "no"}:
            return False


def _ask_status(tty, identifier: str, expected: str):
    while True:
        tty.write(f"\n{identifier}: {expected}\nStatus [pass/fail/unsupported]: ")
        status = tty.readline().strip().lower()
        if status in {"pass", "fail", "unsupported"}:
            break
    issue = None
    notes = ""
    if status == "fail":
        tty.write("Focused GitHub issue URL containing this exact evidence: ")
        issue = tty.readline().strip()
    if status == "unsupported":
        tty.write("Why is this unsupported in the named path? ")
        notes = tty.readline().strip()
    return status, issue, notes


def _manual_expected(identifier: str) -> str:
    expected = {
        "core-open-edit-save-quit": "Open/edit/save/quit succeeds and the saved fixture matches the intended text.",
        "input-delivery": "Text, navigation, and control shortcuts reach Catomic correctly.",
        "shifted-text": "Uppercase, shifted punctuation, and Unicode are delivered exactly.",
        "fallback-function-keys": "F1 opens help and F2 opens the command prompt.",
        "mouse-mapping": "A click places the cursor at the intended document character.",
        "bracketed-paste": "Terminal paste arrives once as one undoable edit.",
        "osc52": "Copy reaches the host clipboard through OSC 52 without terminal corruption.",
        "resize": "Narrower and wider resizes redraw correctly and preserve cursor/content.",
        "signals": "A deliberately tested signal path exits/suspends safely and restores the terminal.",
        "terminal-restoration": "Normal screen, cursor, echo, input, mouse, and paste modes return.",
    }
    return expected[identifier]


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (EvidenceError, OSError, subprocess.SubprocessError) as error:
        print(f"terminal compatibility failed: {error}", file=sys.stderr)
        raise SystemExit(2)
