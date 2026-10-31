#!/usr/bin/env python3
"""Reject altered references, unsafe archives and credential inheritance."""

from __future__ import annotations

import hashlib
import io
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from scripts import rehearse_frozen_verifier as rehearsal


class FrozenReferenceTest(unittest.TestCase):
    def test_frozen_executables_are_checked_before_loading_reference_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "scripts/regression_verifier_source_guard.py"
            path.parent.mkdir()
            path.write_text("raise RuntimeError('unreviewed code executed')\n")
            with patch.object(rehearsal, "FROZEN_FILES", {str(path.relative_to(root)): "0" * 64}):
                with self.assertRaisesRegex(ValueError, "frozen verifier file changed"):
                    rehearsal.verify_reference(root)

    def test_frozen_files_cannot_disappear_or_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = b"reviewed benchmark\n"
            with patch.object(rehearsal, "FROZEN_FILES", {"checker.py": hashlib.sha256(payload).hexdigest()}):
                with self.assertRaisesRegex(ValueError, "missing or unsafe"):
                    rehearsal.verify_frozen_files(root)
                (root / "checker.py").write_bytes(payload)
                rehearsal.verify_frozen_files(root)
                (root / "checker.py").write_bytes(payload + b"changed\n")
                with self.assertRaisesRegex(ValueError, "changed"):
                    rehearsal.verify_frozen_files(root)

    def test_current_runtime_can_change_without_rewriting_historical_benchmark(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            benchmark = root / rehearsal.BENCHMARK / "check.py"
            benchmark.parent.mkdir(parents=True)
            benchmark.write_bytes(b"published benchmark")
            runtime = root / "scripts/runtime.py"
            runtime.parent.mkdir(parents=True)
            runtime.write_bytes(b"security repair")
            frozen = {str(benchmark.relative_to(root)): hashlib.sha256(benchmark.read_bytes()).hexdigest(), "scripts/runtime.py": "0" * 64}
            with patch.object(rehearsal, "FROZEN_FILES", frozen):
                rehearsal.verify_frozen_files(root, benchmark_only=True)
                with self.assertRaisesRegex(ValueError, "runtime.py"):
                    rehearsal.verify_frozen_files(root)
                benchmark.write_bytes(b"rewritten test")
                with self.assertRaisesRegex(ValueError, "check.py"):
                    rehearsal.verify_frozen_files(root, benchmark_only=True)

    def test_both_original_source_scopes_are_enforced(self):
        with patch.object(rehearsal, "verify_frozen_files"), patch.object(rehearsal.runpy, "run_path") as load:
            root = Path("reference")
            for changed in ("worker-build", "signing-runtime"):
                with self.subTest(scope=changed):
                    load.return_value = {"source_digest": lambda _, scope: "0" * 64 if scope == changed else rehearsal.SOURCE_DIGESTS[scope]}
                    with self.assertRaisesRegex(ValueError, changed):
                        rehearsal.verify_reference(root)

    def test_credentials_and_runtime_overrides_are_not_inherited(self):
        with patch.dict(os.environ, {
            "BASE_KEEPER_PRIVATE_KEY": "test-only", "AWS_SECRET_ACCESS_KEY": "test-only",
            "GITHUB_TOKEN": "test-only", "DATABASE_URL": "test-only",
            "ARTIFACT_S3_SECRET_ACCESS_KEY": "test-only", "PYTHONPATH": "untrusted",
            "GIT_CONFIG_COUNT": "1", "WORKSPACE_ROOT": "untrusted", "LD_PRELOAD": "untrusted",
        }, clear=True):
            environment = rehearsal.clean_environment()
        for key in ("BASE_KEEPER_PRIVATE_KEY", "AWS_SECRET_ACCESS_KEY", "GITHUB_TOKEN", "DATABASE_URL", "ARTIFACT_S3_SECRET_ACCESS_KEY", "PYTHONPATH", "GIT_CONFIG_COUNT", "WORKSPACE_ROOT", "LD_PRELOAD"):
            self.assertNotIn(key, environment)
        self.assertEqual(environment["GIT_TERMINAL_PROMPT"], "0")
        self.assertEqual(environment["GIT_CONFIG_GLOBAL"], os.devnull)

    def archive(self, base: Path, entries: list[tuple[str, bytes, bytes]]) -> Path:
        path = base / "source.tar"
        with tarfile.open(path, "w") as bundle:
            for name, kind, payload in entries:
                info = tarfile.TarInfo(name)
                info.type = kind
                info.size = len(payload) if kind == tarfile.REGTYPE else 0
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                    info.linkname = "../outside"
                bundle.addfile(info, io.BytesIO(payload) if info.size else None)
        return path

    def test_extracts_exact_bytes_without_checkout_line_ending_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            output = base / "extracted"
            output.mkdir()
            payload = b"line one\r\nline two\n\x00"
            archive = self.archive(base, [("crates/input", tarfile.REGTYPE, payload)])
            rehearsal.extract_reference(archive, output)
            self.assertEqual((output / "crates/input").read_bytes(), payload)

    def test_unsafe_archives_are_rejected_before_any_file_is_written(self):
        cases = [
            ("../outside", tarfile.REGTYPE), ("/absolute", tarfile.REGTYPE),
            ("C:/absolute", tarfile.REGTYPE), ("a/../outside", tarfile.REGTYPE),
            ("a\\outside", tarfile.REGTYPE), ("a//b", tarfile.REGTYPE),
            ("symlink", tarfile.SYMTYPE), ("hardlink", tarfile.LNKTYPE),
            ("device", tarfile.CHRTYPE), ("safe", tarfile.REGTYPE),
            ("SAFE", tarfile.REGTYPE),
        ]
        for name, kind in cases:
            with self.subTest(name=name, kind=kind), tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                output = base / "extracted"
                output.mkdir()
                archive = self.archive(base, [("safe", tarfile.REGTYPE, b"first"), (name, kind, b"bad")])
                with self.assertRaisesRegex(ValueError, "unsafe or duplicate"):
                    rehearsal.extract_reference(archive, output)
                self.assertEqual(list(output.iterdir()), [])

    def test_archive_member_limit_is_enforced(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            output = base / "extracted"
            output.mkdir()
            archive = self.archive(base, [("one", tarfile.REGTYPE, b"a"), ("two", tarfile.REGTYPE, b"b")])
            with patch.object(rehearsal, "MAX_FILES", 1), self.assertRaisesRegex(ValueError, "expansion exceeds"):
                rehearsal.extract_reference(archive, output)
            self.assertEqual(list(output.iterdir()), [])

    def test_missing_or_corrupt_fixture_cannot_start_rehearsal(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            with patch.object(rehearsal.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "missing or unsafe"):
                    rehearsal.stage_reference(base, base / "reference")
                fixture = base / rehearsal.REFERENCE_FIXTURE
                fixture.parent.mkdir(parents=True)
                fixture.write_bytes(b"corrupted fixture")
                with self.assertRaisesRegex(ValueError, "fixture digest mismatch"):
                    rehearsal.stage_reference(base, base / "reference")
                run.assert_not_called()

    def test_bundled_reference_is_exact_without_git_or_network(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(rehearsal.subprocess, "run") as run:
            reference = Path(directory) / "reference"
            rehearsal.stage_reference(rehearsal.ROOT, reference)
            run.assert_not_called()
            (reference / "crates/worker/src/main.rs").write_text("changed worker")
            with self.assertRaisesRegex(ValueError, "worker-build source digest mismatch"):
                rehearsal.verify_reference(reference)


if __name__ == "__main__":
    unittest.main()
