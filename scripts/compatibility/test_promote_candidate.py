#!/usr/bin/env python3
"""Regression tests for acceptance-candidate promotion into a release."""

from __future__ import annotations

import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

from compatlib import EvidenceError, sha256_file
from promote_candidate import CANDIDATE_SCHEMA, promote_candidate
from test_compatlib import fixture_result


SOURCE_SHA = "c" * 40


class CandidatePromotionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.candidate = self.root / "candidate"
        self.candidate.mkdir()
        binary = self.candidate / "catomic"
        binary.write_bytes(b"accepted managed binary bytes")
        digest = sha256_file(binary)
        (self.candidate / "catomic.sha256").write_text(
            f"{digest}  catomic\n", encoding="ascii"
        )
        (self.candidate / "candidate-build.json").write_text(
            json.dumps(
                {
                    "schema_version": CANDIDATE_SCHEMA,
                    "source_sha": SOURCE_SHA,
                    "source_dirty": False,
                    "managed_release": True,
                }
            ),
            encoding="utf-8",
        )
        artifact = {
            "commit": SOURCE_SHA,
            "release": None,
            "binary_name": "catomic",
            "binary_sha256": digest,
            "binary_size": binary.stat().st_size,
            "version_output": "catomic 0.1.0 (commit cccccccccccc)",
        }
        records = []
        for index, (kind, identifier) in enumerate(
            (
                ("terminal", "direct-pty"),
                ("terminal", "tmux"),
                ("filesystem", "runner-filesystem"),
                ("filesystem", "tmpfs"),
            )
        ):
            record = fixture_result()
            record["artifact"] = artifact.copy()
            record["run"]["id"] = f"run-{index}"
            record["environment"]["kind"] = kind
            record["environment"]["id"] = identifier
            records.append(record)
        matrix = {
            "schema_version": "catomic-compatibility-matrix-v1",
            "generated_at_utc": "2026-09-07T00:00:00Z",
            "release_candidate_gate": "not-requested",
            "artifact": artifact,
            "results": records,
        }
        (self.candidate / "matrix.json").write_text(
            json.dumps(matrix), encoding="utf-8"
        )

    def tearDown(self):
        self.temporary.cleanup()

    def test_promotes_only_byte_identical_managed_candidate(self):
        output = self.root / "release" / "catomic"
        digest = promote_candidate(self.candidate, SOURCE_SHA, output)
        self.assertEqual(output.read_bytes(), b"accepted managed binary bytes")
        self.assertEqual(sha256_file(output), digest)
        self.assertEqual(output.stat().st_mode & 0o777, 0o755)

    def test_rejects_binary_changed_after_acceptance(self):
        (self.candidate / "catomic").write_bytes(b"different bytes")
        with self.assertRaisesRegex(EvidenceError, "candidate checksum differs"):
            promote_candidate(self.candidate, SOURCE_SHA, self.root / "output")

    def test_rejects_matrix_for_different_binary(self):
        path = self.candidate / "matrix.json"
        matrix = json.loads(path.read_text(encoding="utf-8"))
        matrix["artifact"]["binary_sha256"] = "d" * 64
        for record in matrix["results"]:
            record["artifact"]["binary_sha256"] = "d" * 64
        path.write_text(json.dumps(matrix), encoding="utf-8")
        with self.assertRaisesRegex(EvidenceError, "matrix checksum differs"):
            promote_candidate(self.candidate, SOURCE_SHA, self.root / "output")

    def test_rejects_unmanaged_or_wrong_source_metadata(self):
        path = self.candidate / "candidate-build.json"
        metadata = json.loads(path.read_text(encoding="utf-8"))
        metadata["managed_release"] = False
        path.write_text(json.dumps(metadata), encoding="utf-8")
        with self.assertRaisesRegex(EvidenceError, "clean managed build"):
            promote_candidate(self.candidate, SOURCE_SHA, self.root / "output")

        metadata["managed_release"] = True
        metadata["source_sha"] = "d" * 40
        path.write_text(json.dumps(metadata), encoding="utf-8")
        with self.assertRaisesRegex(EvidenceError, "clean managed build"):
            promote_candidate(self.candidate, SOURCE_SHA, self.root / "output")

    def test_rejects_failed_automated_result(self):
        path = self.candidate / "matrix.json"
        matrix = json.loads(path.read_text(encoding="utf-8"))
        matrix["results"][0]["scenarios"][0]["status"] = "fail"
        matrix["results"][0]["scenarios"][0]["focused_issue"] = (
            "https://github.com/maelguimet/catomic/issues/123"
        )
        matrix["results"][0]["overall_status"] = "fail"
        path.write_text(json.dumps(matrix), encoding="utf-8")
        with self.assertRaisesRegex(EvidenceError, "failed automated result"):
            promote_candidate(self.candidate, SOURCE_SHA, self.root / "output")

    def test_rejects_falsified_release_candidate_gate(self):
        path = self.candidate / "matrix.json"
        matrix = json.loads(path.read_text(encoding="utf-8"))
        matrix["release_candidate_gate"] = "pass"
        path.write_text(json.dumps(matrix), encoding="utf-8")
        with self.assertRaisesRegex(EvidenceError, "unexpected release-candidate gate"):
            promote_candidate(self.candidate, SOURCE_SHA, self.root / "output")


class ReleaseWorkflowOrderingTests(unittest.TestCase):
    def test_acceptance_download_keeps_cargo_source_clean(self):
        repository = Path(__file__).resolve().parents[2]
        workflow = (repository / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )
        promotion = workflow.split(
            "- name: Promote the exact accepted managed candidate", 1
        )[1].split("- name:", 1)[0]
        destination = re.search(r"--dir ([^\s]+)", promotion).group(1)
        result = subprocess.run(
            ["git", "check-ignore", "--no-index", "--quiet", "--", f"{destination}/catomic"],
            cwd=repository,
            check=False,
        )
        self.assertEqual(result.returncode, 0, "downloaded candidate would dirty Cargo source")

    def test_acceptance_candidate_is_validated_before_tag_creation(self):
        repository = Path(__file__).resolve().parents[2]
        workflow = (repository / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )
        bind = workflow.index("- name: Bind the run to the source and package version")
        promote = workflow.index("- name: Promote the exact accepted managed candidate")
        create_tag = workflow.index("- name: Create immutable tag for the accepted candidate")
        verify_tag = workflow.index("- name: Verify the immutable tag identity")
        self.assertLess(bind, promote)
        self.assertLess(promote, create_tag)
        self.assertLess(create_tag, verify_tag)
        self.assertLess(workflow.index("promote_candidate.py"), workflow.index("git tag"))

    def test_daily_driver_validates_acceptance_bundle_before_session_launch(self):
        repository = Path(__file__).resolve().parents[2]
        gate = (repository / "scripts/daily-driver-gate.sh").read_text(encoding="utf-8")
        self.assertIn("--acceptance-bundle", gate)
        promote = gate.index("promote_candidate.py")
        session_start = gate.index('started="$(date -u')
        editor_launch = gate.index('"$release_binary" notes.txt')
        self.assertLess(promote, session_start)
        self.assertLess(promote, editor_launch)


if __name__ == "__main__":
    unittest.main()
