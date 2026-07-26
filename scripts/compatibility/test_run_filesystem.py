#!/usr/bin/env python3

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import run_filesystem
from compatlib import scenario
from filesystem_boundaries import BOUNDARY_EXPECTATIONS
from filesystem_scenarios import CORE_FILESYSTEM_SCENARIOS
from run_filesystem import run_filesystem_scenarios


FAILURE_ISSUE = "https://github.com/maelguimet/catomic/issues/232"


class FilesystemFailureContinuationTests(unittest.TestCase):
    def test_core_failure_is_recorded_and_remaining_scenario_runs(self):
        calls = []

        def fail_core(candidate: Path, root: Path):
            calls.append(("atomic-save", candidate, root))
            raise RuntimeError("exact bytes differed")

        def pass_boundary(candidate: Path, root: Path):
            calls.append(("symlink-save", candidate, root))
            return scenario(
                "symlink-save",
                BOUNDARY_EXPECTATIONS["symlink-save"],
                "pass",
                exit_status=0,
                before_sha256=None,
                after_sha256=None,
                evidence=["remaining scenario ran"],
            )

        scenarios = (
            ("atomic-save", fail_core),
            ("symlink-save", pass_boundary),
        )
        candidate = Path("/candidate")
        with tempfile.TemporaryDirectory() as directory:
            sandbox = Path(directory)
            with patch.object(run_filesystem, "SCENARIOS", scenarios):
                records = run_filesystem_scenarios(
                    candidate, sandbox, FAILURE_ISSUE
                )

        self.assertEqual(
            [record["id"] for record in records],
            ["atomic-save", "symlink-save"],
        )
        self.assertEqual(
            records[0]["expected"],
            CORE_FILESYSTEM_SCENARIOS["atomic-save"],
        )
        self.assertEqual(records[0]["status"], "fail")
        self.assertEqual(records[0]["focused_issue"], FAILURE_ISSUE)
        self.assertEqual(
            records[0]["evidence"],
            ["scenario exception: RuntimeError: exact bytes differed"],
        )
        self.assertEqual(records[1]["status"], "pass")
        self.assertEqual([call[0] for call in calls], ["atomic-save", "symlink-save"])

    def test_failure_issue_does_not_mask_unknown_scenario_identifier(self):
        def fail_unknown(_candidate: Path, _root: Path):
            raise RuntimeError("unknown scenario failed")

        with tempfile.TemporaryDirectory() as directory:
            with patch.object(
                run_filesystem,
                "SCENARIOS",
                (("unknown-scenario", fail_unknown),),
            ):
                with self.assertRaisesRegex(KeyError, "unknown-scenario"):
                    run_filesystem_scenarios(
                        Path("/candidate"),
                        Path(directory),
                        FAILURE_ISSUE,
                    )


if __name__ == "__main__":
    unittest.main()
