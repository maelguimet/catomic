#!/usr/bin/env python3
"""Purpose: validate one Catomic compatibility environment result.
Owns: evidence field types, hash/status invariants, and focused-failure linkage.
Must not: discover environments, execute scenarios, write files, or contact a network.
Invariants: every accepted failure links a focused issue and overall status is derived.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import re
from typing import Any


SCHEMA_VERSION = "catomic-compatibility-v1"
SHA40 = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
ISSUE_URL = re.compile(r"^https://github\.com/[^/]+/[^/]+/issues/[1-9][0-9]*$")
STATUSES = {"pass", "fail", "unsupported"}


class EvidenceError(ValueError):
    """A compatibility record violates the evidence contract."""


def validate_result(record: dict[str, Any]) -> None:
    if record.get("schema_version") != SCHEMA_VERSION:
        raise EvidenceError("unknown schema_version")
    _require_mapping(record, "run", "artifact", "environment")
    _validate_run(record["run"])
    _validate_artifact(record["artifact"])
    _validate_environment(record["environment"])
    scenarios = record.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        raise EvidenceError("scenarios must be a non-empty list")
    identifiers: set[str] = set()
    for item in scenarios:
        _validate_scenario(item)
        if item["id"] in identifiers:
            raise EvidenceError(f"duplicate scenario id: {item['id']}")
        identifiers.add(item["id"])
    statuses = {item["status"] for item in scenarios}
    expected = (
        "fail"
        if "fail" in statuses
        else "pass"
        if "pass" in statuses
        else "unsupported"
    )
    if record.get("overall_status") != expected:
        raise EvidenceError(f"overall_status must be {expected}")


def _require_mapping(parent: dict[str, Any], *keys: str) -> None:
    for key in keys:
        if not isinstance(parent.get(key), dict):
            raise EvidenceError(f"{key} must be an object")


def _require_strings(parent: dict[str, Any], *keys: str) -> None:
    for key in keys:
        if not isinstance(parent.get(key), str) or not parent[key]:
            raise EvidenceError(f"{key} must be a non-empty string")


def _validate_run(run: dict[str, Any]) -> None:
    _require_strings(run, "id", "started_at_utc", "finished_at_utc", "operator")


def _validate_artifact(item: dict[str, Any]) -> None:
    _require_strings(item, "commit", "binary_name", "binary_sha256", "version_output")
    if not SHA40.fullmatch(item["commit"]):
        raise EvidenceError("artifact commit is malformed")
    if not SHA256.fullmatch(item["binary_sha256"]):
        raise EvidenceError("artifact binary_sha256 is malformed")
    if not isinstance(item.get("binary_size"), int) or item["binary_size"] <= 0:
        raise EvidenceError("artifact binary_size must be positive")
    if item.get("release") is not None and not isinstance(item["release"], str):
        raise EvidenceError("artifact release must be a string or null")


def _validate_environment(item: dict[str, Any]) -> None:
    _require_strings(item, "kind", "id")
    if item["kind"] not in {"terminal", "filesystem"}:
        raise EvidenceError("environment kind must be terminal or filesystem")
    _require_mapping(item, "host", "terminal")
    _require_mapping(item["host"], "os", "locale", "filesystem")
    _require_strings(item["host"], "kernel", "architecture")
    _require_strings(
        item["host"]["filesystem"], "type", "mount_target", "timestamp_mode"
    )
    terminal = item["terminal"]
    _require_strings(
        terminal, "path", "emulator", "emulator_version", "TERM", "dimensions"
    )
    if terminal.get("category") not in {"pty", "multiplexer", "remote", "gui"}:
        raise EvidenceError("terminal category is invalid")
    if not isinstance(terminal.get("manual"), bool):
        raise EvidenceError("terminal manual must be boolean")


def _validate_scenario(item: dict[str, Any]) -> None:
    if not isinstance(item, dict):
        raise EvidenceError("scenario must be an object")
    _require_strings(item, "id", "expected", "status")
    if item["status"] not in STATUSES:
        raise EvidenceError(f"invalid scenario status: {item['status']}")
    if item.get("exit_status") is not None and not isinstance(item["exit_status"], int):
        raise EvidenceError("scenario exit_status must be an integer or null")
    for key in ("before_sha256", "after_sha256"):
        value = item.get(key)
        if value is not None and (
            not isinstance(value, str) or not SHA256.fullmatch(value)
        ):
            raise EvidenceError(f"scenario {key} is malformed")
    if not isinstance(item.get("evidence"), list) or not all(
        isinstance(value, str) and value for value in item["evidence"]
    ):
        raise EvidenceError("scenario evidence must contain non-empty strings")
    issue = item.get("focused_issue")
    if item["status"] == "fail" and (
        not isinstance(issue, str) or not ISSUE_URL.fullmatch(issue)
    ):
        raise EvidenceError(
            f"failed scenario {item['id']} must link a focused GitHub issue"
        )
    if item["status"] == "unsupported" and not item.get("notes"):
        raise EvidenceError(f"unsupported scenario {item['id']} must explain why")
