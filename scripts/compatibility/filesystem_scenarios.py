#!/usr/bin/env python3
"""Purpose: exercise Catomic's atomic save, conflict, and recovery filesystem paths.
Owns: exact file transitions, frozen-mtime rewrites, abrupt interruption, and hashes.
Must not: test special inode types, publish evidence, use ambient config, or use a network.
Invariants: disk bytes are asserted at every dangerous confirmation boundary.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import os
import signal
import time
from pathlib import Path

from compatlib import scenario, sha256_file
from pty_driver import PtyError, PtyProcess, isolated_environment


CORE_FILESYSTEM_SCENARIOS = {
    "atomic-save": "Saving replaces the inode atomically, preserves exact bytes, and exits 0.",
    "external-same-size-frozen": "A same-size rewrite with restored mtime is detected; first save refuses and the unchanged second save confirms.",
    "external-different-size": "A different-size rewrite is detected; first save refuses and the unchanged second save confirms.",
    "recovery-after-interruption": "SIGKILL leaves source unchanged; the private sidecar is previewed, recovered, and explicitly saved.",
}


def atomic_save(binary: Path, root: Path):
    fixture = root / "atomic-save.txt"
    fixture.write_text("base", encoding="utf-8")
    before = sha256_file(fixture)
    inode_before = fixture.stat().st_ino
    child = _spawn(binary, fixture, root / "atomic-env")
    with child:
        child.wait_for(b"base")
        child.send(b"X\x13\x11")
        exit_status = child.finish()
    after = sha256_file(fixture)
    if exit_status != 0 or fixture.read_text(encoding="utf-8") != "Xbase":
        raise PtyError("atomic save did not write exact expected bytes")
    inode_after = fixture.stat().st_ino
    if inode_before == inode_after:
        raise PtyError("atomic save did not replace the destination inode")
    return scenario(
        "atomic-save",
        CORE_FILESYSTEM_SCENARIOS["atomic-save"],
        "pass",
        exit_status=exit_status,
        before_sha256=before,
        after_sha256=after,
        evidence=[f"inode changed from {inode_before} to {inode_after}"],
    )


def same_size_frozen_conflict(binary: Path, root: Path):
    return _external_conflict(
        binary,
        root,
        scenario_id="external-same-size-frozen",
        external=b"REPLACED",
        freeze_mtime=True,
    )


def different_size_conflict(binary: Path, root: Path):
    return _external_conflict(
        binary,
        root,
        scenario_id="external-different-size",
        external=b"external-different-size",
        freeze_mtime=False,
    )


def _external_conflict(
    binary: Path,
    root: Path,
    *,
    scenario_id: str,
    external: bytes,
    freeze_mtime: bool,
):
    fixture = root / f"{scenario_id}.txt"
    fixture.write_bytes(b"ORIGINAL")
    baseline_stat = fixture.stat()
    before = sha256_file(fixture)
    child = _spawn(binary, fixture, root / f"{scenario_id}-env")
    with child:
        child.wait_for(b"ORIGINAL")
        child.send(b"L")
        child.wait_for(b"LORIGINAL")
        with fixture.open("r+b") as stream:
            stream.write(external)
            stream.truncate()
            stream.flush()
            os.fsync(stream.fileno())
        if freeze_mtime:
            os.utime(
                fixture,
                ns=(baseline_stat.st_atime_ns, baseline_stat.st_mtime_ns),
            )
            if fixture.stat().st_mtime_ns != baseline_stat.st_mtime_ns:
                raise PtyError("filesystem did not restore the requested frozen mtime")
        external_hash = sha256_file(fixture)
        child.send(b"\x13")
        child.wait_for(b"File changed on disk. Press Ctrl+S again to overwrite.")
        if fixture.read_bytes() != external:
            raise PtyError("first conflict save overwrote the external revision")
        child.send(b"\x13\x11")
        exit_status = child.finish()
    after = sha256_file(fixture)
    if exit_status != 0 or fixture.read_bytes() != b"LORIGINAL":
        raise PtyError("confirmed conflict save did not write the local revision")
    evidence = [
        f"external revision sha256={external_hash}",
        "first Ctrl+S preserved the external revision",
        "second Ctrl+S was bound to the unchanged observed revision",
    ]
    if freeze_mtime:
        evidence.append(f"mtime frozen at {baseline_stat.st_mtime_ns} ns")
    return scenario(
        scenario_id,
        CORE_FILESYSTEM_SCENARIOS[scenario_id],
        "pass",
        exit_status=exit_status,
        before_sha256=before,
        after_sha256=after,
        evidence=evidence,
    )


def recovery_after_interruption(binary: Path, root: Path):
    fixture = root / "recovery.txt"
    fixture.write_text("disk", encoding="utf-8")
    before = sha256_file(fixture)
    environment = isolated_environment(root / "recovery-env")
    config = Path(environment["XDG_CONFIG_HOME"]) / "catomic" / "config.toml"
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(
        "[recovery]\nenabled = true\ninterval_secs = 5\nmax_bytes = 1024\n",
        encoding="utf-8",
    )
    sidecar = fixture.with_name(f"{fixture.name}.catnap")
    child = PtyProcess([str(binary.resolve(strict=True)), str(fixture)], environment)
    with child:
        child.wait_for(b"disk")
        child.send(b"\x1b[200~RECOVERED-\x1b[201~")
        child.wait_for(b"RECOVERED-disk")
        _wait_for_sidecar(child, sidecar, b"RECOVERED-disk")
        child.signal(signal.SIGKILL)
        killed_status = child.finish()
    if killed_status != -signal.SIGKILL or fixture.read_bytes() != b"disk":
        raise PtyError("interruption did not leave the source unchanged")
    sidecar_hash = sha256_file(sidecar)

    restarted = PtyProcess(
        [str(binary.resolve(strict=True)), str(fixture)], environment
    )
    with restarted:
        restarted.wait_for(b"Catnap recovery found. Run :recover to preview it.")
        restarted.send(b"\x1bOQ")
        restarted.wait_for(b"Command:")
        restarted.send(b"recover\r")
        restarted.wait_for(b"Catnap preview (read-only). Enter recovers; Esc cancels.")
        if fixture.read_bytes() != b"disk":
            raise PtyError("opening recovery preview changed the source")
        restarted.send(b"\r")
        restarted.wait_for(b"Catnap recovered; Ctrl+Z undoes it")
        if fixture.read_bytes() != b"disk":
            raise PtyError("applying recovery preview saved without confirmation")
        restarted.send(b"\x13\x11")
        exit_status = restarted.finish()
    after = sha256_file(fixture)
    if (
        exit_status != 0
        or fixture.read_bytes() != b"RECOVERED-disk"
        or sidecar.exists()
    ):
        raise PtyError(
            "explicit recovery save did not commit exact bytes and remove sidecar"
        )
    return scenario(
        "recovery-after-interruption",
        CORE_FILESYSTEM_SCENARIOS["recovery-after-interruption"],
        "pass",
        exit_status=exit_status,
        before_sha256=before,
        after_sha256=after,
        evidence=[
            f"first process exit status={killed_status}",
            f"catnap sha256={sidecar_hash}",
            "source hash stayed unchanged through interruption, preview, and apply",
        ],
    )


def _spawn(binary: Path, fixture: Path, environment_root: Path) -> PtyProcess:
    environment = isolated_environment(environment_root)
    return PtyProcess([str(binary.resolve(strict=True)), str(fixture)], environment)


def _wait_for_sidecar(
    child: PtyProcess, sidecar: Path, expected: bytes, timeout: float = 10.0
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        child.read_available(0.05)
        try:
            if sidecar.read_bytes() == expected:
                return
        except FileNotFoundError:
            pass
    raise PtyError("timed out waiting for the exact catnap sidecar")
