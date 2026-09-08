#!/usr/bin/env python3
"""Verify and copy the exact managed binary covered by Acceptance evidence.

The release workflow uses this boundary instead of rebuilding a tested candidate.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
from pathlib import Path
from typing import Any

from build_report import validate_aggregate
from compatlib import EvidenceError, sha256_file
from result_validation import SHA40, SHA256, validate_result


CANDIDATE_SCHEMA = "catomic-managed-candidate-v1"
AUTOMATED_ENVIRONMENTS = {
    ("terminal", "direct-pty"),
    ("terminal", "tmux"),
    ("filesystem", "runner-filesystem"),
    ("filesystem", "tmpfs"),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate_directory", type=Path)
    parser.add_argument("source_sha")
    parser.add_argument("output_binary", type=Path)
    return parser.parse_args()


def promote_candidate(candidate_directory: Path, source_sha: str, output: Path) -> str:
    if not SHA40.fullmatch(source_sha):
        raise EvidenceError("source SHA must be 40 lowercase hexadecimal characters")
    if candidate_directory.is_symlink() or not candidate_directory.is_dir():
        raise EvidenceError("candidate directory must be a non-symlink directory")

    candidate = _regular_file(candidate_directory / "catomic", "candidate binary")
    checksum = _candidate_checksum(candidate_directory / "catomic.sha256")
    actual_checksum = sha256_file(candidate)
    if checksum != actual_checksum:
        raise EvidenceError("candidate checksum differs from candidate binary bytes")

    metadata = _object(candidate_directory / "candidate-build.json", "build metadata")
    if set(metadata) != {
        "schema_version",
        "source_sha",
        "source_dirty",
        "managed_release",
    }:
        raise EvidenceError("candidate build metadata fields are invalid")
    if metadata != {
        "schema_version": CANDIDATE_SCHEMA,
        "source_sha": source_sha,
        "source_dirty": False,
        "managed_release": True,
    }:
        raise EvidenceError(
            "candidate was not recorded as a clean managed build of the release source"
        )

    matrix = _object(candidate_directory / "matrix.json", "compatibility matrix")
    if set(matrix) != {
        "schema_version",
        "generated_at_utc",
        "release_candidate_gate",
        "artifact",
        "results",
    } or matrix.get("schema_version") != "catomic-compatibility-matrix-v1":
        raise EvidenceError("compatibility matrix fields are invalid")
    if matrix["release_candidate_gate"] != "not-requested":
        raise EvidenceError("Acceptance matrix has an unexpected release-candidate gate")
    results = matrix.get("results")
    if not isinstance(results, list) or not results:
        raise EvidenceError("compatibility matrix must contain results")
    for record in results:
        validate_result(record)
        if record["overall_status"] == "fail":
            raise EvidenceError("Acceptance matrix contains a failed automated result")
    validate_aggregate(results)
    environments = {
        (record["environment"]["kind"], record["environment"]["id"])
        for record in results
    }
    if environments != AUTOMATED_ENVIRONMENTS:
        raise EvidenceError(
            "Acceptance matrix does not contain the exact automated environments"
        )
    if matrix.get("artifact") != results[0]["artifact"]:
        raise EvidenceError("compatibility matrix artifact differs from its results")

    artifact = matrix["artifact"]
    if artifact["commit"] != source_sha:
        raise EvidenceError("compatibility matrix source SHA differs from release source")
    if artifact["binary_name"] != "catomic":
        raise EvidenceError("compatibility matrix names an unexpected candidate binary")
    if artifact["binary_sha256"] != actual_checksum:
        raise EvidenceError("compatibility matrix checksum differs from candidate binary bytes")
    if artifact["binary_size"] != candidate.stat().st_size:
        raise EvidenceError("compatibility matrix size differs from candidate binary bytes")

    output.parent.mkdir(parents=True, exist_ok=True)
    try:
        with candidate.open("rb") as source, output.open("xb") as destination:
            shutil.copyfileobj(source, destination, length=1024 * 1024)
            destination.flush()
            os.fsync(destination.fileno())
    except FileExistsError as error:
        raise EvidenceError(f"refusing to overwrite promoted binary: {output}") from error
    output.chmod(0o755)
    if sha256_file(output) != actual_checksum:
        raise EvidenceError("promoted binary bytes differ from accepted candidate")
    return actual_checksum


def _regular_file(path: Path, label: str) -> Path:
    if path.is_symlink() or not path.is_file():
        raise EvidenceError(f"{label} must be a non-symlink regular file")
    return path


def _object(path: Path, label: str) -> dict[str, Any]:
    path = _regular_file(path, label)
    try:
        value = json.loads(path.read_text(encoding="utf-8", errors="strict"))
    except (json.JSONDecodeError, UnicodeError) as error:
        raise EvidenceError(f"{label} is not valid UTF-8 JSON") from error
    if not isinstance(value, dict):
        raise EvidenceError(f"{label} must be a JSON object")
    return value


def _candidate_checksum(path: Path) -> str:
    path = _regular_file(path, "candidate checksum")
    line = path.read_text(encoding="ascii", errors="strict")
    match = re.fullmatch(r"([0-9a-f]{64})  catomic\n", line)
    if match is None or not SHA256.fullmatch(match.group(1)):
        raise EvidenceError("candidate checksum file is malformed")
    return match.group(1)


def main() -> int:
    args = parse_args()
    digest = promote_candidate(
        args.candidate_directory.resolve(strict=True),
        args.source_sha,
        args.output_binary,
    )
    print(f"promoted accepted managed candidate sha256: {digest}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (EvidenceError, OSError) as error:
        print(f"candidate promotion failed: {error}", file=sys.stderr)
        raise SystemExit(1)
