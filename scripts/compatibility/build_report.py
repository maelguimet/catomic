#!/usr/bin/env python3
"""Purpose: validate and aggregate compatibility results for one exact Catomic artifact.
Owns: cross-result identity checks, release-candidate gates, JSON bundle, and Markdown matrix.
Must not: execute tests, overwrite evidence, publish artifacts, or contact a network.
Invariants: one report contains one checksum/commit and cannot bury unlinked failures.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any

from compatlib import EvidenceError, load_result, utc_now


TERMINAL_REQUIRED = {
    "core-open-edit-save-quit",
    "input-delivery",
    "shifted-text",
    "fallback-function-keys",
    "mouse-mapping",
    "bracketed-paste",
    "osc52",
    "resize",
    "signals",
    "terminal-restoration",
}
GUI_MANUAL_REQUIRED = {
    "input-delivery",
    "shifted-text",
    "fallback-function-keys",
    "mouse-mapping",
    "bracketed-paste",
}
FILESYSTEM_REQUIRED = {
    "atomic-save",
    "external-same-size-frozen",
    "external-different-size",
    "recovery-after-interruption",
}
FILESYSTEM_BOUNDARIES = {
    "symlink-save",
    "read-only-refusal",
    "hard-link-save",
    "xattr-save",
    "acl-save",
    "non-regular-refusal",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", nargs="+", type=Path)
    parser.add_argument("--output-json", type=Path, required=True)
    parser.add_argument("--output-markdown", type=Path, required=True)
    parser.add_argument("--release-candidate", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    records = [load_result(path) for path in args.results]
    validate_aggregate(records)
    gate_errors = release_candidate_errors(records) if args.release_candidate else []
    if gate_errors:
        raise EvidenceError(
            "release-candidate gate failed:\n- " + "\n- ".join(gate_errors)
        )
    bundle = {
        "schema_version": "catomic-compatibility-matrix-v1",
        "generated_at_utc": utc_now(),
        "release_candidate_gate": "pass" if args.release_candidate else "not-requested",
        "artifact": records[0]["artifact"],
        "results": records,
    }
    _write_new(args.output_json, json.dumps(bundle, indent=2, sort_keys=True) + "\n")
    _write_new(args.output_markdown, markdown_report(bundle, args.results))
    print(f"matrix JSON: {args.output_json}")
    print(f"matrix Markdown: {args.output_markdown}")
    print(
        f"results: {len(records)}; artifact sha256: {bundle['artifact']['binary_sha256']}"
    )
    return 0


def validate_aggregate(records: list[dict[str, Any]]) -> None:
    if not records:
        raise EvidenceError("at least one result is required")
    identity = records[0]["artifact"]
    seen_runs: set[str] = set()
    seen_environments: set[tuple[str, str]] = set()
    for record in records:
        if record["artifact"] != identity:
            raise EvidenceError("all results must name the same exact artifact record")
        run_id = record["run"]["id"]
        if run_id in seen_runs:
            raise EvidenceError(f"duplicate run id: {run_id}")
        seen_runs.add(run_id)
        environment = record["environment"]
        key = (environment["kind"], environment["id"])
        if key in seen_environments:
            raise EvidenceError(f"duplicate environment id: {key[0]}/{key[1]}")
        seen_environments.add(key)


def release_candidate_errors(records: list[dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    for record in records:
        if record["overall_status"] == "fail":
            environment = record["environment"]
            errors.append(
                f"{environment['kind']} {environment['id']} contains failed scenarios"
            )
    terminals = [
        record for record in records if record["environment"]["kind"] == "terminal"
    ]
    qualifying = [
        record
        for record in terminals
        if _passes(record, "core-open-edit-save-quit")
        and _passes(record, "terminal-restoration")
    ]
    qualifying_paths = {_material_terminal_identity(record) for record in qualifying}
    if len(qualifying_paths) < 3:
        errors.append(
            "three materially different terminal paths must pass core flow and restoration"
        )
    for record in terminals:
        missing = TERMINAL_REQUIRED - _scenario_ids(record)
        if missing:
            errors.append(
                f"terminal {record['environment']['id']} lacks scenarios: {', '.join(sorted(missing))}"
            )
    gui = [
        record
        for record in terminals
        if record["environment"]["terminal"]["manual"]
        and record["environment"]["terminal"]["category"] == "gui"
        and all(_passes(record, identifier) for identifier in GUI_MANUAL_REQUIRED)
    ]
    gui_emulators = {
        record["environment"]["terminal"]["emulator"].strip().casefold()
        for record in gui
    }
    if len(gui_emulators) < 2:
        errors.append(
            "two real GUI terminal paths must manually pass input/shortcut delivery"
        )
    filesystem_records = [
        record for record in records if record["environment"]["kind"] == "filesystem"
    ]
    for record in filesystem_records:
        missing = (FILESYSTEM_REQUIRED | FILESYSTEM_BOUNDARIES) - _scenario_ids(record)
        if missing:
            errors.append(
                f"filesystem {record['environment']['id']} lacks scenarios: {', '.join(sorted(missing))}"
            )
    for filesystem_type in ("ext4", "tmpfs"):
        matches = [
            record
            for record in filesystem_records
            if record["environment"]["host"]["filesystem"]["type"] == filesystem_type
            and record["environment"]["host"]["filesystem"]["timestamp_mode"]
            == "frozen-mtime"
            and all(_passes(record, identifier) for identifier in FILESYSTEM_REQUIRED)
        ]
        if not matches:
            errors.append(
                f"{filesystem_type} must pass atomic save, conflicts, and recovery"
            )
    return errors


def markdown_report(bundle: dict[str, Any], paths: list[Path]) -> str:
    artifact = bundle["artifact"]
    lines = [
        "# Catomic compatibility matrix result",
        "",
        f"- Commit: `{artifact['commit']}`",
        f"- Binary: `{artifact['binary_name']}` ({artifact['binary_size']} bytes)",
        f"- SHA-256: `{artifact['binary_sha256']}`",
        f"- Version: `{artifact['version_output']}`",
        f"- Release-candidate gate: `{bundle['release_candidate_gate']}`",
        "",
        "| Kind | Environment | OS / kernel | Filesystem | Terminal path | TERM | Result | Scenarios | Source |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for record, path in zip(bundle["results"], paths):
        environment = record["environment"]
        host = environment["host"]
        terminal = environment["terminal"]
        counts = Counter(item["status"] for item in record["scenarios"])
        summary = ", ".join(
            f"{key}={counts[key]}"
            for key in ("pass", "fail", "unsupported")
            if counts[key]
        )
        lines.append(
            "| "
            + " | ".join(
                _cell(value)
                for value in (
                    environment["kind"],
                    environment["id"],
                    f"{host['os'].get('pretty_name', 'unknown')} / {host['kernel']}",
                    f"{host['filesystem']['type']} at {host['filesystem']['mount_target']}",
                    f"{terminal['path']}: {terminal['emulator']} {terminal['emulator_version']}",
                    terminal["TERM"],
                    record["overall_status"],
                    summary,
                    path.name,
                )
            )
            + " |"
        )
    lines.extend(
        [
            "",
            "Every row's complete environment, expected results, exit statuses, hashes, restoration evidence, and focused issue links are retained in the JSON bundle.",
            "",
        ]
    )
    return "\n".join(lines)


def _scenario_ids(record: dict[str, Any]) -> set[str]:
    return {item["id"] for item in record["scenarios"]}


def _passes(record: dict[str, Any], identifier: str) -> bool:
    return any(
        item["id"] == identifier and item["status"] == "pass"
        for item in record["scenarios"]
    )


def _material_terminal_identity(record: dict[str, Any]) -> tuple[str, ...]:
    terminal = record["environment"]["terminal"]
    return tuple(
        str(terminal[key]).strip().casefold()
        for key in ("category", "emulator", "multiplexer", "ssh_path")
    )


def _write_new(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("x", encoding="utf-8", errors="strict", newline="\n") as stream:
            stream.write(text)
    except FileExistsError as error:
        raise EvidenceError(f"refusing to overwrite report: {path}") from error


def _cell(value: object) -> str:
    return str(value).replace("|", "\\|").replace("\n", " ")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except EvidenceError as error:
        print(f"compatibility report failed: {error}", file=sys.stderr)
        raise SystemExit(2)
