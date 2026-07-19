#!/usr/bin/env python3
"""Purpose: exercise symlink, permission, metadata, link, and non-regular boundaries.
Owns: fail-closed save fixtures and exact preservation assertions for special targets.
Must not: test conflict/recovery policy, publish evidence, use ambient config, or network.
Invariants: every refused save leaves bytes, inode semantics, and metadata fixtures intact.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import os
import shutil
import socket
import subprocess
from pathlib import Path

from compatlib import scenario, sha256_file
from pty_driver import PtyError, PtyProcess, isolated_environment


BOUNDARY_EXPECTATIONS = {
    "symlink-save": "Saving through a final symlink preserves the link and atomically replaces its regular referent.",
    "read-only-refusal": "Saving a read-only regular file fails and preserves its bytes and mode.",
    "hard-link-refusal": "Saving a multiply linked file fails and preserves both directory entries and bytes.",
    "xattr-refusal": "Saving a file with a user xattr fails and preserves bytes and the attribute.",
    "acl-refusal": "Saving a file with a POSIX ACL fails and preserves bytes and the ACL.",
    "non-regular-refusal": "FIFO, directory, and Unix-socket targets are refused without blocking or replacement.",
}


def symlink_save(binary: Path, root: Path):
    target = root / "symlink-target.txt"
    link = root / "symlink.txt"
    target.write_text("base", encoding="utf-8")
    link.symlink_to(target.name)
    before = sha256_file(target)
    child = _spawn(binary, link, root / "symlink-env")
    with child:
        child.wait_for(b"base")
        child.send(b"X\x13\x11")
        exit_status = child.finish()
    after = sha256_file(target)
    if exit_status != 0 or not link.is_symlink() or target.read_bytes() != b"Xbase":
        raise PtyError(
            "symlink save changed the link or wrote unexpected referent bytes"
        )
    return scenario(
        "symlink-save",
        BOUNDARY_EXPECTATIONS["symlink-save"],
        "pass",
        exit_status=exit_status,
        before_sha256=before,
        after_sha256=after,
        evidence=[f"link target remained {os.readlink(link)}"],
    )


def read_only_refusal(binary: Path, root: Path):
    target = root / "read-only.txt"
    target.write_text("readonly", encoding="utf-8")
    target.chmod(0o444)
    try:
        return _refused_save(
            binary,
            root,
            target,
            "read-only-refusal",
            b"Save error: refusing to replace",
        )
    finally:
        target.chmod(0o644)


def hard_link_refusal(binary: Path, root: Path):
    target = root / "hard-link.txt"
    peer = root / "hard-link-peer.txt"
    target.write_text("shared", encoding="utf-8")
    os.link(target, peer)
    record = _refused_save(
        binary,
        root,
        target,
        "hard-link-refusal",
        b"Save error: refusing atomic save",
    )
    if target.stat().st_ino != peer.stat().st_ino or peer.read_bytes() != b"shared":
        raise PtyError("hard-link refusal changed the peer or inode identity")
    record["evidence"].append(f"both entries retained inode {target.stat().st_ino}")
    return record


def xattr_refusal(binary: Path, root: Path):
    target = root / "xattr.txt"
    target.write_text("attributed", encoding="utf-8")
    try:
        os.setxattr(target, b"user.catomic-compat", b"preserve-me")
    except OSError as error:
        return _unsupported(
            "xattr-refusal", f"filesystem cannot set a user xattr: {error}"
        )
    record = _refused_save(
        binary,
        root,
        target,
        "xattr-refusal",
        b"Save error: refusing atomic save",
    )
    if os.getxattr(target, b"user.catomic-compat") != b"preserve-me":
        raise PtyError("xattr refusal did not preserve the user attribute")
    record["evidence"].append("user.catomic-compat=preserve-me remained present")
    return record


def acl_refusal(binary: Path, root: Path):
    if shutil.which("setfacl") is None or shutil.which("getfacl") is None:
        return _unsupported("acl-refusal", "setfacl/getfacl are not installed")
    target = root / "acl.txt"
    target.write_text("acl", encoding="utf-8")
    completed = subprocess.run(
        ["setfacl", "-m", "u:65534:r--", str(target)],
        text=True,
        capture_output=True,
        timeout=5,
        check=False,
    )
    if completed.returncode != 0:
        return _unsupported(
            "acl-refusal",
            f"filesystem cannot set POSIX ACL: {completed.stderr.strip()}",
        )
    before_acl = _getfacl(target)
    record = _refused_save(
        binary,
        root,
        target,
        "acl-refusal",
        b"Save error: refusing atomic save",
    )
    if _getfacl(target) != before_acl:
        raise PtyError("ACL refusal changed the access ACL")
    record["evidence"].append("getfacl output remained byte-identical")
    return record


def non_regular_refusal(binary: Path, root: Path):
    fifo = root / "target.fifo"
    directory = root / "target-directory"
    socket_path = root / "s"
    os.mkfifo(fifo, 0o600)
    directory.mkdir()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(socket_path))
    statuses = []
    try:
        for index, target in enumerate((fifo, directory, socket_path)):
            child = _spawn(binary, target, root / f"nonregular-env-{index}")
            with child:
                child.wait_for(b"refusing to open non-regular file")
                statuses.append(child.finish())
    finally:
        listener.close()
    if (
        statuses != [1, 1, 1]
        or not fifo.is_fifo()
        or not directory.is_dir()
        or not socket_path.exists()
    ):
        raise PtyError(f"non-regular refusal statuses or targets differed: {statuses}")
    return scenario(
        "non-regular-refusal",
        BOUNDARY_EXPECTATIONS["non-regular-refusal"],
        "pass",
        exit_status=1,
        before_sha256=None,
        after_sha256=None,
        evidence=[f"FIFO/directory/socket exit statuses={statuses}"],
    )


def _refused_save(
    binary: Path,
    root: Path,
    target: Path,
    scenario_id: str,
    expected_error: bytes,
):
    before = sha256_file(target)
    inode_before = target.stat().st_ino
    child = _spawn(binary, target, root / f"{scenario_id}-env")
    with child:
        child.wait_for(target.read_bytes())
        child.send(b"X\x13")
        child.wait_for(expected_error)
        if sha256_file(target) != before:
            raise PtyError("refused first save changed target bytes")
        child.send(b"\x11\x11")
        exit_status = child.finish()
    after = sha256_file(target)
    if exit_status != 0 or before != after or target.stat().st_ino != inode_before:
        raise PtyError("refused save changed bytes/inode or did not quit cleanly")
    return scenario(
        scenario_id,
        BOUNDARY_EXPECTATIONS[scenario_id],
        "pass",
        exit_status=exit_status,
        before_sha256=before,
        after_sha256=after,
        evidence=[f"refusal preserved inode {inode_before}"],
    )


def _spawn(binary: Path, fixture: Path, environment_root: Path) -> PtyProcess:
    return PtyProcess(
        [str(binary.resolve(strict=True)), str(fixture)],
        isolated_environment(environment_root),
    )


def _unsupported(identifier: str, notes: str):
    return scenario(
        identifier,
        BOUNDARY_EXPECTATIONS[identifier],
        "unsupported",
        exit_status=None,
        before_sha256=None,
        after_sha256=None,
        evidence=[notes],
        notes=notes,
    )


def _getfacl(path: Path) -> str:
    return subprocess.run(
        ["getfacl", "-cp", str(path)],
        check=True,
        text=True,
        capture_output=True,
        timeout=5,
    ).stdout
