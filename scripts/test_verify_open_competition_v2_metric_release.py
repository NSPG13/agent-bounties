import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("verify-open-competition-v2-metric-release.py")
SPEC = importlib.util.spec_from_file_location("metric_release", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class MetricReleaseCandidateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "source.txt").write_text("canonical GMV source\n", encoding="utf-8")
        self.source_hash = MODULE.canonical_source_hash(self.root, ("source.txt",))
        self.vkey = "0x" + "11" * 32
        self.elf_hash = "0x" + "22" * 32
        self.schema_hash = "0x" + "33" * 32
        self.metric_hash = "0x" + "44" * 32
        identity = {
            "status": "awaiting_reproduction",
            "sp1_commit": MODULE.EXPECTED_SP1_COMMIT,
            "sp1_runtime_commit": MODULE.EXPECTED_SP1_RUNTIME_COMMIT,
            "program_vkey": None,
            "source_hash": None,
            "elf_keccak256": None,
            "elf_sha256": None,
        }
        (self.root / "identity.json").write_text(json.dumps(identity), encoding="utf-8")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def evidence(self, release_candidate: bool = True) -> dict[str, object]:
        journal = bytearray(MODULE.JOURNAL_BYTES)
        journal[MODULE.PROGRAM_VKEY_WORD * 32 : (MODULE.PROGRAM_VKEY_WORD + 1) * 32] = bytes.fromhex(
            self.vkey[2:]
        )
        journal[MODULE.SOURCE_HASH_WORD * 32 : (MODULE.SOURCE_HASH_WORD + 1) * 32] = bytes.fromhex(
            self.source_hash[2:]
        )
        journal[MODULE.ELF_HASH_WORD * 32 : (MODULE.ELF_HASH_WORD + 1) * 32] = bytes.fromhex(
            self.elf_hash[2:]
        )
        journal[MODULE.JOURNAL_SCHEMA_WORD * 32 : (MODULE.JOURNAL_SCHEMA_WORD + 1) * 32] = bytes.fromhex(
            self.schema_hash[2:]
        )
        journal[MODULE.METRIC_PROGRAM_WORD * 32 : (MODULE.METRIC_PROGRAM_WORD + 1) * 32] = bytes.fromhex(
            self.metric_hash[2:]
        )
        return {
            "mode": "execute",
            "release_candidate": release_candidate,
            "program_vkey": self.vkey,
            "elf_keccak256": self.elf_hash,
            "elf_sha256": "55" * 32,
            "journal_hex": "0x" + journal.hex(),
            "cycles": 123,
        }

    def run_verifier(self, release_candidate: bool = True) -> subprocess.CompletedProcess[str]:
        evidence = self.evidence(release_candidate)
        for name in ("first.jsonl", "second.jsonl"):
            (self.root / name).write_text(
                "cargo-prove sp1 (f6a2dff test)\n" + json.dumps(evidence) + "\n",
                encoding="utf-8",
            )
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--first",
                str(self.root / "first.jsonl"),
                "--second",
                str(self.root / "second.jsonl"),
                "--root",
                str(self.root),
                "--output",
                str(self.root / "release.json"),
                "--profile-id",
                "canonical-gmv-attribution-metric-v1",
                "--identity-path",
                "identity.json",
                "--source-file",
                "source.txt",
                "--journal-schema-hash",
                self.schema_hash,
                "--metric-program-hash",
                self.metric_hash,
                "--candidate",
            ],
            text=True,
            capture_output=True,
            check=False,
        )

    def test_candidate_reproduction_emits_nonproduction_identity(self) -> None:
        result = self.run_verifier()
        self.assertEqual(result.returncode, 0, result.stderr)
        release = json.loads((self.root / "release.json").read_text(encoding="utf-8"))
        self.assertEqual(release["classification"], "candidate_reproduction")
        self.assertEqual(release["source_hash"], self.source_hash)
        self.assertEqual(release["program_vkey"], self.vkey)

    def test_candidate_requires_hydrated_execution_evidence(self) -> None:
        result = self.run_verifier(release_candidate=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("release-candidate execution evidence", result.stderr)


if __name__ == "__main__":
    unittest.main()
