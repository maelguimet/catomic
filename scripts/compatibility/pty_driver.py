#!/usr/bin/env python3
"""Purpose: drive one exact Catomic binary through a real Linux pseudo-terminal.
Owns: PTY spawn, input/output, resize, signals, timeouts, and exit-status capture.
Must not: interpret editor policy, write evidence, use ambient config, or use a network.
Invariants: children are killed/reaped on failure and output reads stay bounded by timeout.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import errno
import fcntl
import os
import pty
import select
import signal
import struct
import termios
import time
from pathlib import Path


class PtyError(RuntimeError):
    """The PTY child timed out or violated a scenario expectation."""


class PtyProcess:
    def __init__(
        self,
        argv: list[str],
        environment: dict[str, str],
        *,
        rows: int = 24,
        columns: int = 80,
        cwd: Path | None = None,
    ) -> None:
        self.argv = argv
        self.output = bytearray()
        self.wait_status: int | None = None
        pid, master = pty.fork()
        if pid == 0:
            if cwd is not None:
                os.chdir(cwd)
            os.execvpe(argv[0], argv, environment)
        self.pid = pid
        self.master = master
        self.resize(rows, columns)

    def __enter__(self) -> "PtyProcess":
        return self

    def __exit__(self, _kind, _value, _traceback) -> None:
        self.close()

    def resize(self, rows: int, columns: int) -> None:
        if rows <= 0 or columns <= 0:
            raise PtyError("PTY dimensions must be positive")
        size = struct.pack("HHHH", rows, columns, 0, 0)
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, size)

    def send(self, data: bytes) -> None:
        view = memoryview(data)
        while view:
            written = os.write(self.master, view)
            view = view[written:]

    def signal(self, signal_number: int) -> None:
        os.kill(self.pid, signal_number)

    def read_available(self, timeout: float = 0.05) -> bytes:
        readable, _, _ = select.select([self.master], [], [], timeout)
        if not readable:
            self._poll_exit()
            return b""
        try:
            chunk = os.read(self.master, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                self._poll_exit()
                return b""
            raise
        self.output.extend(chunk)
        self._poll_exit()
        return chunk

    def wait_for(self, expected: bytes, timeout: float = 5.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if expected in self.output:
                return
            self.read_available(min(0.05, max(0.0, deadline - time.monotonic())))
            if self.wait_status is not None and expected not in self.output:
                break
        excerpt = bytes(self.output[-1000:]).decode("utf-8", errors="replace")
        raise PtyError(f"timed out waiting for {expected!r}; output tail: {excerpt!r}")

    def wait_for_more_output(self, previous_length: int, timeout: float = 5.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.read_available(0.05)
            if len(self.output) > previous_length:
                return
        raise PtyError("timed out waiting for output after terminal resize")

    def finish(self, timeout: float = 10.0) -> int:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and self.wait_status is None:
            self.read_available(0.05)
        if self.wait_status is None:
            self._kill_and_reap()
            raise PtyError(f"child timed out: {self.argv!r}")
        self._drain()
        return os.waitstatus_to_exitcode(self.wait_status)

    def output_text(self) -> str:
        return bytes(self.output).decode("utf-8", errors="replace")

    def close(self) -> None:
        if getattr(self, "master", None) is None:
            return
        if self.wait_status is None:
            self._kill_and_reap()
        try:
            os.close(self.master)
        except OSError:
            pass
        self.master = None

    def _poll_exit(self) -> None:
        if self.wait_status is not None:
            return
        finished, status = os.waitpid(self.pid, os.WNOHANG)
        if finished:
            self.wait_status = status

    def _drain(self) -> None:
        while True:
            readable, _, _ = select.select([self.master], [], [], 0)
            if not readable:
                return
            try:
                chunk = os.read(self.master, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    return
                raise
            if not chunk:
                return
            self.output.extend(chunk)

    def _kill_and_reap(self) -> None:
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            _, self.wait_status = os.waitpid(self.pid, 0)
        except ChildProcessError:
            pass


def isolated_environment(root: Path, term: str = "xterm-256color") -> dict[str, str]:
    environment = os.environ.copy()
    for name in ("home", "config", "data", "state"):
        (root / name).mkdir(parents=True, exist_ok=True)
    environment.update(
        {
            "HOME": str(root / "home"),
            "XDG_CONFIG_HOME": str(root / "config"),
            "XDG_DATA_HOME": str(root / "data"),
            "XDG_STATE_HOME": str(root / "state"),
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "TERM": term,
        }
    )
    return environment


def restoration_evidence(output: bytes) -> dict[str, object]:
    sequences = {
        "mouse_disabled": b"\x1b[?1000l" in output,
        "bracketed_paste_disabled": b"\x1b[?2004l" in output,
        "alternate_screen_left": b"\x1b[?1049l" in output,
    }
    return {
        "restored": all(sequences.values()),
        "stty_before": None,
        "stty_after": None,
        "teardown_sequences": sequences,
    }
