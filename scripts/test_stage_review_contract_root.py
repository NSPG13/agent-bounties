from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from stage_review_contract_root import REQUIRED_SOURCES, StageError, stage_contract_root


class StageReviewContractRootTests(unittest.TestCase):
    def make_worktree(self, root: Path) -> Path:
        worktree = root / "worktree"
        for index, relative in enumerate(REQUIRED_SOURCES):
            source = worktree / relative
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(f"// source {index}\n", encoding="utf-8")
        return worktree

    def test_stages_exact_regular_sources_with_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worktree = self.make_worktree(root)
            output = root / "staged"

            report = stage_contract_root(worktree, output)

            self.assertEqual([item["path"] for item in report], [p.as_posix() for p in REQUIRED_SOURCES])
            self.assertTrue(all(len(str(item["sha256"])) == 64 for item in report))
            for relative in REQUIRED_SOURCES:
                self.assertEqual(
                    (output / relative).read_bytes(),
                    (worktree / relative).read_bytes(),
                )

    def test_rejects_missing_required_source(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worktree = self.make_worktree(root)
            (worktree / REQUIRED_SOURCES[0]).unlink()

            with self.assertRaisesRegex(StageError, "missing"):
                stage_contract_root(worktree, root / "staged")

    def test_rejects_oversized_or_binary_source(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worktree = self.make_worktree(root)
            (worktree / REQUIRED_SOURCES[0]).write_bytes(b"12345")
            with self.assertRaisesRegex(StageError, "exceeds"):
                stage_contract_root(worktree, root / "oversized", max_source_bytes=4)

            (worktree / REQUIRED_SOURCES[0]).write_bytes(b"source\x00data")
            with self.assertRaisesRegex(StageError, "NUL"):
                stage_contract_root(worktree, root / "binary")

    def test_rejects_output_inside_untrusted_worktree(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worktree = self.make_worktree(root)

            with self.assertRaisesRegex(StageError, "outside"):
                stage_contract_root(worktree, worktree / "staged")

    def test_rejects_symlink_source_when_supported(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worktree = self.make_worktree(root)
            source = worktree / REQUIRED_SOURCES[0]
            target = root / "outside.rs"
            target.write_text("// outside\n", encoding="utf-8")
            source.unlink()
            try:
                source.symlink_to(target)
            except OSError:
                self.skipTest("symlink creation is unavailable")

            with self.assertRaisesRegex(StageError, "symlink"):
                stage_contract_root(worktree, root / "staged")

    def test_review_scripts_pass_the_staged_contract_root(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        engine = (repo_root / "scripts/review_external_pr.py").read_text(encoding="utf-8")

        self.assertIn("stage_review_contract_root.py", engine)
        self.assertIn('pr_data["baseRefOid"]', engine)
        self.assertIn('"--worktree",', engine)
        self.assertIn('base / "Cargo.toml"', engine)
        self.assertIn('"--contract-root",', engine)
        self.assertNotIn('"--contract-root",\n                    ROOT', engine)


if __name__ == "__main__":
    unittest.main()
