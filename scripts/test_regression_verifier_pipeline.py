from __future__ import annotations

import importlib.util
import io
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("regression_verifier_pipeline.py")
SPEC = importlib.util.spec_from_file_location("regression_verifier_pipeline", SCRIPT)
assert SPEC and SPEC.loader
pipeline = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = pipeline
SPEC.loader.exec_module(pipeline)


def archive(entries: list[tuple[str, bytes | None, str]]) -> bytes:
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as bundle:
        for name, body, kind in entries:
            info = tarfile.TarInfo(name)
            if kind == "dir":
                info.type = tarfile.DIRTYPE
                bundle.addfile(info)
            elif kind == "symlink":
                info.type = tarfile.SYMTYPE
                info.linkname = "target"
                bundle.addfile(info)
            else:
                assert body is not None
                info.size = len(body)
                bundle.addfile(info, io.BytesIO(body))
    return output.getvalue()


class RegressionVerifierPipelineTests(unittest.TestCase):
    def test_artifact_reference_requires_an_exact_public_commit(self) -> None:
        expected = ("owner/repo", "a" * 40)
        self.assertEqual(
            pipeline.parse_github_commit_url(
                f"https://github.com/owner/repo/commit/{'a' * 40}"
            ),
            expected,
        )
        invalid = [
            f"http://github.com/owner/repo/commit/{'a' * 40}",
            f"https://user@github.com/owner/repo/commit/{'a' * 40}",
            f"https://github.com/owner/repo/commit/{'a' * 39}",
            f"https://github.com/owner/repo/commit/{'a' * 40}?download=1",
            f"https://evil.example/owner/repo/commit/{'a' * 40}",
        ]
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(pipeline.PipelineError):
                pipeline.parse_github_commit_url(value)

    def test_safe_archive_strips_root_and_selects_committed_subdirectory(self) -> None:
        value = archive(
            [
                ("repo-root/bench", None, "dir"),
                ("repo-root/bench/test.txt", b"expected", "file"),
                ("repo-root/source.txt", b"ignored", "file"),
            ]
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pipeline.extract_snapshot(
                value,
                root,
                subdirectory="bench",
                max_bytes=100,
                max_files=2,
            )
            self.assertEqual((root / "test.txt").read_bytes(), b"expected")
            self.assertFalse((root / "source.txt").exists())

    def test_archive_links_traversal_and_size_overrun_fail_closed(self) -> None:
        cases = [
            archive([("root/link", None, "symlink")]),
            archive([("root/../escape", b"bad", "file")]),
        ]
        for value in cases:
            with self.subTest(size=len(value)), tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(pipeline.PipelineError):
                    pipeline.extract_snapshot(
                        value,
                        Path(temporary),
                        subdirectory=None,
                        max_bytes=100,
                        max_files=2,
                    )

        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(pipeline.PipelineError):
                pipeline.extract_snapshot(
                    archive([("root/large", b"0123456789", "file")]),
                    Path(temporary),
                    subdirectory=None,
                    max_bytes=5,
                    max_files=2,
                )

    def test_benchmark_source_is_exact_and_commit_pinned(self) -> None:
        job = {
            "terms": {
                "document": {
                    "benchmark": {
                        "source": {
                            "kind": "github_commit",
                            "repository": "owner/repo",
                            "commit": "b" * 40,
                            "subdirectory": "benchmarks/task",
                        }
                    }
                }
            }
        }
        self.assertEqual(
            pipeline.benchmark_source(job),
            ("owner/repo", "b" * 40, "benchmarks/task"),
        )
        job["terms"]["document"]["benchmark"]["source"]["branch"] = "main"
        with self.assertRaises(pipeline.PipelineError):
            pipeline.benchmark_source(job)


if __name__ == "__main__":
    unittest.main()
