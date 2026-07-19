#!/usr/bin/env python3
"""Purpose: build, validate, and write Catomic compatibility evidence records.
Owns: artifact/host discovery, result invariants, hashes, and immutable JSON writes.
Must not: launch Catomic, mutate test fixtures, contact a network, or publish evidence.
Invariants: valid failures link a focused issue and records bind one exact binary/commit.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import hashlib
import json
import locale
import os
import platform
import stat
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from result_validation import EvidenceError, SCHEMA_VERSION, SHA40, validate_result


def utc_now() -> str:
    return (
        datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stage_artifact(binary: Path, sandbox: Path) -> Path:
    """Copy the candidate into the private sandbox before collecting evidence."""
    if binary.is_symlink():
        raise EvidenceError("binary must be a non-symlink regular file")
    source = binary.resolve(strict=True)
    source_stat = source.stat()
    if not source.is_file():
        raise EvidenceError("binary must be a non-symlink regular file")
    destination = sandbox / "candidate" / source.name
    destination.parent.mkdir(mode=0o700)
    with source.open("rb") as input_stream, destination.open("xb") as output_stream:
        for chunk in iter(lambda: input_stream.read(1024 * 1024), b""):
            output_stream.write(chunk)
        output_stream.flush()
        os.fsync(output_stream.fileno())
    destination.chmod(stat.S_IMODE(source_stat.st_mode))
    return destination


def artifact(binary: Path, commit: str, release: str | None) -> dict[str, Any]:
    if binary.is_symlink():
        raise EvidenceError("binary must be a non-symlink regular file")
    binary = binary.resolve(strict=True)
    if not binary.is_file():
        raise EvidenceError("binary must be a non-symlink regular file")
    if not SHA40.fullmatch(commit):
        raise EvidenceError("commit must be 40 lowercase hexadecimal characters")
    completed = subprocess.run(
        [str(binary), "--version"],
        check=True,
        text=True,
        capture_output=True,
        timeout=10,
    )
    version = completed.stdout.strip()
    if not version.startswith("catomic "):
        raise EvidenceError(f"unexpected version output: {version!r}")
    return {
        "commit": commit,
        "release": release,
        "binary_name": binary.name,
        "binary_sha256": sha256_file(binary),
        "binary_size": binary.stat().st_size,
        "version_output": version,
    }


def host_environment(probe_path: Path, timestamp_mode: str) -> dict[str, Any]:
    probe_path = probe_path.resolve(strict=True)
    return {
        "os": os_release(),
        "kernel": platform.release(),
        "architecture": platform.machine(),
        "locale": locale_environment(),
        "filesystem": filesystem_environment(probe_path, timestamp_mode),
    }


def os_release() -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        for line in Path("/etc/os-release").read_text(encoding="utf-8").splitlines():
            if "=" not in line:
                continue
            key, value = line.split("=", 1)
            values[key] = value.strip().strip('"')
    except OSError:
        pass
    return {
        "pretty_name": values.get("PRETTY_NAME", platform.platform()),
        "id": values.get("ID", "unknown"),
        "version_id": values.get("VERSION_ID", "unknown"),
    }


def locale_environment() -> dict[str, str]:
    return {
        "LC_ALL": os.environ.get("LC_ALL", ""),
        "LC_CTYPE": os.environ.get("LC_CTYPE", ""),
        "LANG": os.environ.get("LANG", ""),
        "resolved": locale.setlocale(locale.LC_CTYPE),
    }


def filesystem_environment(path: Path, timestamp_mode: str) -> dict[str, str]:
    if timestamp_mode not in {"native", "frozen-mtime"}:
        raise EvidenceError("timestamp mode must be native or frozen-mtime")
    completed = subprocess.run(
        [
            "findmnt",
            "--json",
            "--target",
            str(path),
            "--output",
            "TARGET,SOURCE,FSTYPE,OPTIONS",
        ],
        check=True,
        text=True,
        capture_output=True,
        timeout=10,
    )
    filesystems = json.loads(completed.stdout).get("filesystems", [])
    if len(filesystems) != 1:
        raise EvidenceError(f"findmnt returned {len(filesystems)} entries for {path}")
    item = filesystems[0]
    return {
        "probe_path": str(path),
        "mount_target": str(item.get("target", "unknown")),
        "mount_source": str(item.get("source", "unknown")),
        "type": str(item.get("fstype", "unknown")),
        "mount_options": str(item.get("options", "unknown")),
        "timestamp_mode": timestamp_mode,
    }


def scenario(
    scenario_id: str,
    expected: str,
    status: str,
    *,
    exit_status: int | None,
    before_sha256: str | None,
    after_sha256: str | None,
    evidence: list[str],
    restoration: dict[str, Any] | None = None,
    focused_issue: str | None = None,
    notes: str = "",
) -> dict[str, Any]:
    return {
        "id": scenario_id,
        "expected": expected,
        "status": status,
        "exit_status": exit_status,
        "before_sha256": before_sha256,
        "after_sha256": after_sha256,
        "terminal_restoration": restoration,
        "evidence": evidence,
        "focused_issue": focused_issue,
        "notes": notes,
    }


def result(
    run_id: str,
    started_at: str,
    operator: str,
    artifact_record: dict[str, Any],
    environment: dict[str, Any],
    scenarios: list[dict[str, Any]],
) -> dict[str, Any]:
    statuses = {item["status"] for item in scenarios}
    overall = (
        "fail"
        if "fail" in statuses
        else "pass"
        if "pass" in statuses
        else "unsupported"
    )
    record = {
        "schema_version": SCHEMA_VERSION,
        "run": {
            "id": run_id,
            "started_at_utc": started_at,
            "finished_at_utc": utc_now(),
            "operator": operator,
        },
        "artifact": artifact_record,
        "environment": environment,
        "scenarios": scenarios,
        "overall_status": overall,
    }
    validate_result(record)
    return record


def load_result(path: Path) -> dict[str, Any]:
    try:
        record = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"cannot read {path}: {error}") from error
    validate_result(record)
    return record


def write_new_json(path: Path, record: dict[str, Any]) -> None:
    validate_result(record)
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x", encoding="utf-8") as stream:
            json.dump(record, stream, indent=2, sort_keys=True)
            stream.write("\n")
    except FileExistsError as error:
        raise EvidenceError(f"refusing to overwrite evidence: {path}") from error
