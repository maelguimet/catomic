#!/usr/bin/env python3
"""Purpose: exercise terminal compatibility scenarios through direct PTY or tmux paths.
Owns: exact input bytes, terminal assertions, fixture hashes, and tmux lifecycle.
Must not: prompt an operator, publish evidence, use ambient config, or contact a network.
Invariants: every session has a timeout and all failures preserve the original fixture hash.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import os
import signal
import time
from pathlib import Path

from compatlib import EvidenceError, scenario, sha256_file
from pty_driver import PtyError, isolated_environment, restoration_evidence
from terminal_contract import terminal_expected
from terminal_path import TerminalLauncher


def run_automated_terminal(
    binary: Path,
    path_id: str,
    root: Path,
    failure_issue: str | None,
) -> list[dict[str, object]]:
    launcher = TerminalLauncher(binary, path_id)
    groups = [
        (
            (
                "core-open-edit-save-quit",
                "input-delivery",
                "shifted-text",
                "bracketed-paste",
                "osc52",
                "terminal-restoration",
            ),
            _core_session,
        ),
        (("fallback-function-keys",), _fallback_session),
        (("mouse-mapping",), _mouse_session),
        (("resize",), _resize_session),
        (("signals",), _signal_session),
    ]
    records: list[dict[str, object]] = []
    for identifiers, runner in groups:
        try:
            records.extend(runner(launcher, root))
        except Exception as error:
            launcher.cleanup()
            if failure_issue is None:
                joined = ", ".join(identifiers)
                raise EvidenceError(
                    f"{joined} failed: {error}; create a focused issue, then rerun with --failure-issue"
                ) from error
            for identifier in identifiers:
                records.append(
                    scenario(
                        identifier,
                        terminal_expected(identifier),
                        "fail",
                        exit_status=None,
                        before_sha256=None,
                        after_sha256=None,
                        evidence=[
                            f"scenario exception: {type(error).__name__}: {error}"
                        ],
                        focused_issue=failure_issue,
                        notes="Automated terminal scenario failed.",
                    )
                )
    return records


def _core_session(launcher: TerminalLauncher, root: Path) -> list[dict[str, object]]:
    fixture = root / "terminal-core.txt"
    fixture.write_text("CORE_BASE", encoding="utf-8")
    before = sha256_file(fixture)
    marker = "SHIFTED !@# ÅΩ中🙂"
    child = launcher.spawn("core", fixture, isolated_environment(root / "core"))
    try:
        child.wait_for(b"CORE_BASE", timeout=10)
        child.send(b"\x1b[200~" + marker.encode() + b"\x1b[201~")
        child.wait_for(marker.encode())
        child.send(b"\x01\x03")
        child.wait_for(b"Copied selection.")
        child.send(b"\x1a\x19\x13\x11")
        exit_status = child.finish()
        output = bytes(child.output)
    finally:
        child.close()
        launcher.cleanup()
    after = sha256_file(fixture)
    restore = restoration_evidence(output)
    if exit_status != 0 or fixture.read_text(encoding="utf-8") != marker + "CORE_BASE":
        raise PtyError("core session did not save the exact shifted Unicode marker")
    if not restore["restored"]:
        raise PtyError("core session omitted one or more terminal teardown sequences")
    common = {
        "exit_status": exit_status,
        "before_sha256": before,
        "after_sha256": after,
        "restoration": restore,
        "evidence": ["exact UTF-8 fixture hash checked"],
    }
    records = [
        scenario(identifier, terminal_expected(identifier), "pass", **common)
        for identifier in (
            "core-open-edit-save-quit",
            "input-delivery",
            "shifted-text",
            "bracketed-paste",
            "terminal-restoration",
        )
    ]
    if b"\x1b]52;c;" in output:
        records.append(scenario("osc52", terminal_expected("osc52"), "pass", **common))
    elif launcher.path_id == "tmux":
        records.append(
            scenario(
                "osc52",
                terminal_expected("osc52"),
                "unsupported",
                exit_status=exit_status,
                before_sha256=before,
                after_sha256=after,
                evidence=[
                    "Catomic reported copy, but tmux did not forward OSC 52 to the outer PTY"
                ],
                restoration=restore,
                notes="A headless tmux client cannot attest the host clipboard; verify manually with the configured tmux set-clipboard policy.",
            )
        )
    else:
        raise PtyError("direct PTY transcript omitted the OSC 52 sequence")
    return records


def _fallback_session(
    launcher: TerminalLauncher, root: Path
) -> list[dict[str, object]]:
    fixture = root / "terminal-fallback.txt"
    fixture.write_text("fallback", encoding="utf-8")
    before = sha256_file(fixture)
    child = launcher.spawn("fallback", fixture, isolated_environment(root / "fallback"))
    try:
        child.wait_for(b"fallback")
        child.send(b"\x1bOP")
        child.wait_for(b"Catomic help")
        child.send(b"\x1b")
        child.wait_for(b"Shortcut help closed.")
        child.send(b"\x1bOQ")
        child.wait_for(b"Command:")
        child.send(b"\x1b")
        child.wait_for(b"Prompt cancelled.")
        child.send(b"\x11")
        exit_status = child.finish()
    finally:
        child.close()
        launcher.cleanup()
    after = sha256_file(fixture)
    if exit_status != 0 or before != after:
        raise PtyError("F1/F2 fallback session changed the fixture or exited nonzero")
    return [
        scenario(
            "fallback-function-keys",
            terminal_expected("fallback-function-keys"),
            "pass",
            exit_status=exit_status,
            before_sha256=before,
            after_sha256=after,
            evidence=["F1 help and F2 command prompt rendered"],
            restoration=None,
        )
    ]


def _mouse_session(launcher: TerminalLauncher, root: Path) -> list[dict[str, object]]:
    fixture = root / "terminal-mouse.txt"
    fixture.write_text("alpha\nbeta", encoding="utf-8")
    before = sha256_file(fixture)
    child = launcher.spawn("mouse", fixture, isolated_environment(root / "mouse"))
    try:
        child.wait_for(b"beta")
        child.send(b"\x1b[<0;3;2M\x1b[<0;3;2mX\x13\x11")
        exit_status = child.finish()
    finally:
        child.close()
        launcher.cleanup()
    after = sha256_file(fixture)
    if exit_status != 0 or fixture.read_text(encoding="utf-8") != "alpha\nbeXta":
        raise PtyError("SGR click did not map to row 2, column 3")
    return [
        scenario(
            "mouse-mapping",
            terminal_expected("mouse-mapping"),
            "pass",
            exit_status=exit_status,
            before_sha256=before,
            after_sha256=after,
            evidence=["saved text proves SGR click position"],
            restoration=None,
        )
    ]


def _resize_session(launcher: TerminalLauncher, root: Path) -> list[dict[str, object]]:
    fixture = root / "terminal-resize.txt"
    fixture.write_text("resize\n" * 30, encoding="utf-8")
    before = sha256_file(fixture)
    child = launcher.spawn("resize", fixture, isolated_environment(root / "resize"))
    try:
        child.wait_for(b"resize")
        length = len(child.output)
        child.resize(10, 40)
        child.signal(signal.SIGWINCH)
        time.sleep(0.1)
        child.send(b"\x1b[18~")
        child.wait_for_more_output(length)
        if launcher.path_id == "direct-pty":
            child.wait_for(b"\x1b[10;1H")
        elif launcher.pane_dimensions() != (40, 9):
            raise PtyError("tmux did not propagate the 40x10 client resize")
        length = len(child.output)
        child.resize(30, 100)
        child.signal(signal.SIGWINCH)
        time.sleep(0.1)
        child.send(b"\x1b[18~")
        child.wait_for_more_output(length)
        if launcher.path_id == "direct-pty":
            child.wait_for(b"\x1b[30;1H")
        elif launcher.pane_dimensions() != (100, 29):
            raise PtyError("tmux did not propagate the 100x30 client resize")
        child.send(b"\x11")
        exit_status = child.finish()
    finally:
        child.close()
        launcher.cleanup()
    after = sha256_file(fixture)
    if exit_status != 0 or before != after:
        raise PtyError("resize session changed the fixture or exited nonzero")
    return [
        scenario(
            "resize",
            terminal_expected("resize"),
            "pass",
            exit_status=exit_status,
            before_sha256=before,
            after_sha256=after,
            evidence=["render output followed 40x10 and 100x30 PTY resizes"],
            restoration=None,
        )
    ]


def _signal_session(launcher: TerminalLauncher, root: Path) -> list[dict[str, object]]:
    fixture = root / "terminal-signal.txt"
    fixture.write_text("signal", encoding="utf-8")
    before = sha256_file(fixture)
    child = launcher.spawn("signal", fixture, isolated_environment(root / "signal"))
    try:
        child.wait_for(b"signal")
        target = (
            child.pid if launcher.path_id == "direct-pty" else launcher.signal_target()
        )
        os.kill(target, signal.SIGTERM)
        exit_status = child.finish()
        output = bytes(child.output)
    finally:
        child.close()
        launcher.cleanup()
    after = sha256_file(fixture)
    restore = restoration_evidence(output)
    if launcher.path_id == "direct-pty" and exit_status != 143:
        raise PtyError(f"direct SIGTERM exit was {exit_status}, expected 143")
    if before != after or not restore["restored"]:
        raise PtyError("SIGTERM changed the fixture or omitted teardown sequences")
    return [
        scenario(
            "signals",
            terminal_expected("signals"),
            "pass",
            exit_status=exit_status,
            before_sha256=before,
            after_sha256=after,
            evidence=[f"SIGTERM delivered to Catomic pid {target}"],
            restoration=restore,
        )
    ]
