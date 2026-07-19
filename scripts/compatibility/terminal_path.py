#!/usr/bin/env python3
"""Purpose: launch Catomic through direct PTY or isolated tmux terminal paths.
Owns: tmux command/socket lifecycle, process lookup, and path environment metadata.
Must not: send editor input, evaluate scenarios, write evidence, or contact a network.
Invariants: tmux servers use unique sockets and signal targets resolve to the exact binary.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import os
import platform
import shlex
import subprocess
from pathlib import Path

from compatlib import EvidenceError
from pty_driver import PtyProcess


class TerminalLauncher:
    def __init__(self, binary: Path, path_id: str) -> None:
        self.binary = binary.resolve(strict=True)
        self.path_id = path_id
        self.socket: str | None = None

    def spawn(
        self, label: str, fixture: Path, environment: dict[str, str]
    ) -> PtyProcess:
        if self.path_id == "direct-pty":
            return PtyProcess([str(self.binary), str(fixture)], environment)
        if self.path_id != "tmux":
            raise EvidenceError(f"unsupported automated terminal path: {self.path_id}")
        self.socket = f"catomic-compat-{os.getpid()}-{label}"
        command = shlex.join([str(self.binary), str(fixture)])
        argv = ["tmux", "-L", self.socket, "-f", "/dev/null", "new-session", command]
        return PtyProcess(argv, environment)

    def signal_target(self) -> int:
        if self.path_id == "direct-pty":
            raise EvidenceError("direct signal target is the PTY child")
        if self.socket is None:
            raise EvidenceError("tmux session is not running")
        completed = subprocess.run(
            ["tmux", "-L", self.socket, "list-panes", "-F", "#{pane_pid}"],
            check=True,
            text=True,
            capture_output=True,
            timeout=5,
        )
        pane_pid = int(completed.stdout.strip().splitlines()[0])
        target = _find_executable_descendant(pane_pid, self.binary)
        if target is None:
            raise EvidenceError(
                "could not bind tmux signal scenario to the Catomic process"
            )
        return target

    def pane_dimensions(self) -> tuple[int, int]:
        if self.socket is None:
            raise EvidenceError("tmux session is not running")
        completed = subprocess.run(
            [
                "tmux",
                "-L",
                self.socket,
                "list-panes",
                "-F",
                "#{pane_width} #{pane_height}",
            ],
            check=True,
            text=True,
            capture_output=True,
            timeout=5,
        )
        width, height = completed.stdout.strip().split()
        return int(width), int(height)

    def cleanup(self) -> None:
        if self.socket is None:
            return
        subprocess.run(
            ["tmux", "-L", self.socket, "kill-server"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
            check=False,
        )
        self.socket = None


def terminal_details(path_id: str) -> dict[str, object]:
    python_pty = f"Python {platform.python_version()} stdlib pty"
    if path_id == "direct-pty":
        return {
            "path": path_id,
            "category": "pty",
            "manual": False,
            "emulator": "Linux direct PTY",
            "emulator_version": python_pty,
            "TERM": "xterm-256color",
            "dimensions": "80x24",
            "multiplexer": "none",
            "multiplexer_version": "none",
            "ssh_path": "none",
        }
    version = subprocess.run(
        ["tmux", "-V"], check=True, text=True, capture_output=True, timeout=5
    ).stdout.strip()
    return {
        "path": path_id,
        "category": "multiplexer",
        "manual": False,
        "emulator": "Linux PTY feeding tmux",
        "emulator_version": python_pty,
        "TERM": "tmux-256color",
        "dimensions": "80x23 pane inside 80x24 client",
        "multiplexer": "tmux",
        "multiplexer_version": version,
        "ssh_path": "none",
    }


def _find_executable_descendant(pid: int, binary: Path) -> int | None:
    pending = [pid]
    expected = binary.resolve()
    while pending:
        candidate = pending.pop()
        try:
            if Path(f"/proc/{candidate}/exe").resolve() == expected:
                return candidate
            children = (
                Path(f"/proc/{candidate}/task/{candidate}/children").read_text().split()
            )
            pending.extend(int(value) for value in children)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return None
