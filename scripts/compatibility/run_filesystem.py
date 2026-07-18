#!/usr/bin/env python3
"""Purpose: record artifact-bound filesystem compatibility evidence on one mount.
Owns: safe sandbox lifecycle, scenario orchestration, failure linkage, and result output.
Must not: mount filesystems, overwrite evidence, publish results, or contact a network.
Invariants: all mutations stay under a newly created non-symlink sandbox on the named root.
Phase: post-v0.1 Linux compatibility matrix.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from compatlib import (
    EvidenceError,
    artifact,
    host_environment,
    result,
    scenario,
    utc_now,
    write_new_json,
)
from filesystem_boundaries import (
    BOUNDARY_EXPECTATIONS,
    acl_refusal,
    hard_link_refusal,
    non_regular_refusal,
    read_only_refusal,
    symlink_save,
    xattr_refusal,
)
from filesystem_scenarios import (
    CORE_FILESYSTEM_SCENARIOS,
    atomic_save,
    different_size_conflict,
    recovery_after_interruption,
    same_size_frozen_conflict,
)


SCENARIOS = (
    ("atomic-save", atomic_save),
    ("external-same-size-frozen", same_size_frozen_conflict),
    ("external-different-size", different_size_conflict),
    ("recovery-after-interruption", recovery_after_interruption),
    ("symlink-save", symlink_save),
    ("read-only-refusal", read_only_refusal),
    ("hard-link-refusal", hard_link_refusal),
    ("xattr-refusal", xattr_refusal),
    ("acl-refusal", acl_refusal),
    ("non-regular-refusal", non_regular_refusal),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--release")
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--environment-id", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--operator", required=True)
    parser.add_argument("--failure-issue")
    parser.add_argument("--keep-sandbox", action="store_true")
    parser.add_argument("--run-id", default=f"filesystem-{uuid.uuid4()}")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    started = utc_now()
    if args.root.is_symlink() or not args.root.is_dir():
        raise EvidenceError("root must be an existing non-symlink directory")
    root = args.root.resolve(strict=True)
    sandbox = Path(tempfile.mkdtemp(prefix=".cfc-", dir=root))
    try:
        records = []
        for index, (identifier, runner) in enumerate(SCENARIOS):
            try:
                scenario_root = sandbox / f"s{index}"
                scenario_root.mkdir()
                records.append(runner(args.binary, scenario_root))
            except Exception as error:
                if args.failure_issue is None:
                    raise EvidenceError(
                        f"{identifier} failed: {error}; create a focused issue, then rerun with --failure-issue"
                    ) from error
                expected = CORE_FILESYSTEM_SCENARIOS.get(
                    identifier, BOUNDARY_EXPECTATIONS[identifier]
                )
                records.append(
                    scenario(
                        identifier,
                        expected,
                        "fail",
                        exit_status=None,
                        before_sha256=None,
                        after_sha256=None,
                        evidence=[
                            f"scenario exception: {type(error).__name__}: {error}"
                        ],
                        focused_issue=args.failure_issue,
                        notes="Filesystem compatibility scenario failed.",
                    )
                )
        artifact_record = artifact(args.binary, args.commit, args.release)
        environment = {
            "kind": "filesystem",
            "id": args.environment_id,
            "host": host_environment(sandbox, "frozen-mtime"),
            "terminal": {
                "path": "direct-pty",
                "category": "pty",
                "manual": False,
                "emulator": "Linux direct PTY",
                "emulator_version": f"Python {sys.version_info.major}.{sys.version_info.minor} stdlib pty",
                "TERM": "xterm-256color",
                "dimensions": "80x24",
                "multiplexer": "none",
                "multiplexer_version": "none",
                "ssh_path": "none",
            },
        }
        record = result(
            args.run_id,
            started,
            args.operator,
            artifact_record,
            environment,
            records,
        )
        write_new_json(args.output, record)
        print(f"compatibility result: {args.output}")
        print(f"filesystem: {environment['host']['filesystem']['type']}")
        print(f"artifact sha256: {artifact_record['binary_sha256']}")
        print(f"overall status: {record['overall_status']}")
        return 0 if record["overall_status"] != "fail" else 1
    finally:
        if args.keep_sandbox:
            print(f"sandbox retained: {sandbox}", file=sys.stderr)
        else:
            shutil.rmtree(sandbox, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (EvidenceError, OSError, subprocess.SubprocessError) as error:
        print(f"filesystem compatibility failed: {error}", file=sys.stderr)
        raise SystemExit(2)
