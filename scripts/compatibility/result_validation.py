#!/usr/bin/env python3
"""Purpose: validate one Catomic compatibility environment result.
Owns: evidence field types, hash/status invariants, and focused-failure linkage.
Must not: discover environments, execute scenarios, write files, or contact a network.
Invariants: every accepted failure links a focused issue and overall status is derived.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import re
from datetime import datetime
from typing import Any


SCHEMA_VERSION = "catomic-compatibility-v1"
SHA40 = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
ISSUE_URL = re.compile(r"^https://github\.com/[^/]+/[^/]+/issues/[1-9][0-9]*$")
STATUSES = {"pass", "fail", "unsupported"}


class EvidenceError(ValueError):
    """A compatibility record violates the evidence contract."""


def validate_result(record: dict[str, Any]) -> None:
    _exact_keys(
        record,
        "result",
        {
            "schema_version",
            "run",
            "artifact",
            "environment",
            "scenarios",
            "overall_status",
        },
    )
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


def _exact_keys(item: object, label: str, expected: set[str]) -> None:
    if not isinstance(item, dict):
        raise EvidenceError(f"{label} must be an object")
    actual = set(item)
    if actual != expected:
        missing = sorted(expected - actual)
        unexpected = sorted(actual - expected)
        details = []
        if missing:
            details.append(f"missing {', '.join(missing)}")
        if unexpected:
            details.append(f"unexpected {', '.join(unexpected)}")
        raise EvidenceError(f"{label} fields are invalid: {'; '.join(details)}")


def _require_strings(parent: dict[str, Any], *keys: str) -> None:
    for key in keys:
        if not isinstance(parent.get(key), str) or not parent[key]:
            raise EvidenceError(f"{key} must be a non-empty string")


def _require_text(parent: dict[str, Any], *keys: str) -> None:
    for key in keys:
        if not isinstance(parent.get(key), str):
            raise EvidenceError(f"{key} must be a string")


def _require_date_time(value: str, label: str) -> None:
    try:
        parsed = datetime.fromisoformat(
            value[:-1] + "+00:00" if value.endswith("Z") else value
        )
    except ValueError as error:
        raise EvidenceError(f"{label} must be an ISO 8601 date-time") from error
    if parsed.tzinfo is None:
        raise EvidenceError(f"{label} must include a UTC offset")


def _validate_run(run: dict[str, Any]) -> None:
    _exact_keys(run, "run", {"id", "started_at_utc", "finished_at_utc", "operator"})
    _require_strings(run, "id", "started_at_utc", "finished_at_utc", "operator")
    _require_date_time(run["started_at_utc"], "started_at_utc")
    _require_date_time(run["finished_at_utc"], "finished_at_utc")


def _validate_artifact(item: dict[str, Any]) -> None:
    _exact_keys(
        item,
        "artifact",
        {
            "commit",
            "release",
            "binary_name",
            "binary_sha256",
            "binary_size",
            "version_output",
        },
    )
    _require_strings(item, "commit", "binary_name", "binary_sha256", "version_output")
    if not SHA40.fullmatch(item["commit"]):
        raise EvidenceError("artifact commit is malformed")
    if not SHA256.fullmatch(item["binary_sha256"]):
        raise EvidenceError("artifact binary_sha256 is malformed")
    if (
        not isinstance(item.get("binary_size"), int)
        or isinstance(item["binary_size"], bool)
        or item["binary_size"] <= 0
    ):
        raise EvidenceError("artifact binary_size must be positive")
    if item.get("release") is not None and not isinstance(item["release"], str):
        raise EvidenceError("artifact release must be a string or null")
    if not item["version_output"].startswith("catomic "):
        raise EvidenceError("artifact version_output must start with 'catomic '")


def _validate_environment(item: dict[str, Any]) -> None:
    _exact_keys(item, "environment", {"kind", "id", "host", "terminal"})
    _require_strings(item, "kind", "id")
    if item["kind"] not in {"terminal", "filesystem"}:
        raise EvidenceError("environment kind must be terminal or filesystem")
    _require_mapping(item, "host", "terminal")
    host = item["host"]
    _exact_keys(host, "host", {"os", "kernel", "architecture", "locale", "filesystem"})
    _require_mapping(host, "os", "locale", "filesystem")
    _require_strings(host, "kernel", "architecture")
    _exact_keys(host["os"], "host.os", {"pretty_name", "id", "version_id"})
    _require_text(host["os"], "pretty_name", "id", "version_id")
    _exact_keys(
        host["locale"], "host.locale", {"LC_ALL", "LC_CTYPE", "LANG", "resolved"}
    )
    _require_text(host["locale"], "LC_ALL", "LC_CTYPE", "LANG", "resolved")
    filesystem = host["filesystem"]
    _exact_keys(
        filesystem,
        "host.filesystem",
        {
            "probe_path",
            "mount_target",
            "mount_source",
            "type",
            "mount_options",
            "timestamp_mode",
        },
    )
    _require_text(
        filesystem,
        "probe_path",
        "mount_target",
        "mount_source",
        "type",
        "mount_options",
        "timestamp_mode",
    )
    _require_strings(filesystem, "mount_target", "type")
    if filesystem["timestamp_mode"] not in {"native", "frozen-mtime"}:
        raise EvidenceError("filesystem timestamp_mode is invalid")
    terminal = item["terminal"]
    _exact_keys(
        terminal,
        "terminal",
        {
            "path",
            "category",
            "manual",
            "emulator",
            "emulator_version",
            "TERM",
            "dimensions",
            "multiplexer",
            "multiplexer_version",
            "ssh_path",
        },
    )
    _require_strings(
        terminal, "path", "emulator", "emulator_version", "TERM", "dimensions"
    )
    if terminal.get("category") not in {"pty", "multiplexer", "remote", "gui"}:
        raise EvidenceError("terminal category is invalid")
    if not isinstance(terminal.get("manual"), bool):
        raise EvidenceError("terminal manual must be boolean")
    _require_text(terminal, "multiplexer", "multiplexer_version", "ssh_path")


def _validate_scenario(item: dict[str, Any]) -> None:
    _exact_keys(
        item,
        "scenario",
        {
            "id",
            "expected",
            "status",
            "exit_status",
            "before_sha256",
            "after_sha256",
            "terminal_restoration",
            "evidence",
            "focused_issue",
            "notes",
        },
    )
    _require_strings(item, "id", "expected", "status")
    if item["status"] not in STATUSES:
        raise EvidenceError(f"invalid scenario status: {item['status']}")
    if item.get("exit_status") is not None and (
        not isinstance(item["exit_status"], int)
        or isinstance(item["exit_status"], bool)
    ):
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
    if issue is not None and (
        not isinstance(issue, str) or not ISSUE_URL.fullmatch(issue)
    ):
        raise EvidenceError(f"scenario {item['id']} focused_issue is malformed")
    if item["status"] == "fail" and (
        not isinstance(issue, str) or not ISSUE_URL.fullmatch(issue)
    ):
        raise EvidenceError(
            f"failed scenario {item['id']} must link a focused GitHub issue"
        )
    if item["status"] == "unsupported" and not item.get("notes"):
        raise EvidenceError(f"unsupported scenario {item['id']} must explain why")
    if not isinstance(item.get("notes"), str):
        raise EvidenceError("scenario notes must be a string")
    _validate_restoration(item.get("terminal_restoration"))


def _validate_restoration(item: object) -> None:
    if item is None:
        return
    _exact_keys(
        item,
        "terminal_restoration",
        {"restored", "stty_before", "stty_after", "teardown_sequences"},
    )
    assert isinstance(item, dict)
    if not isinstance(item["restored"], bool):
        raise EvidenceError("terminal_restoration restored must be boolean")
    for key in ("stty_before", "stty_after"):
        if item[key] is not None and not isinstance(item[key], str):
            raise EvidenceError(f"terminal_restoration {key} must be a string or null")
    sequences = item["teardown_sequences"]
    if sequences is None:
        return
    _exact_keys(
        sequences,
        "teardown_sequences",
        {"mouse_disabled", "bracketed_paste_disabled", "alternate_screen_left"},
    )
    assert isinstance(sequences, dict)
    if not all(isinstance(value, bool) for value in sequences.values()):
        raise EvidenceError("teardown sequence evidence must contain booleans")
