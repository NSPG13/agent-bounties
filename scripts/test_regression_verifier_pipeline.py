from __future__ import annotations

import importlib.util
import io
import json
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


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
    def test_runner_selects_single_verifier_and_legacy_two_verifier_jobs(self) -> None:
        configured = ["0x" + "1" * 40, "0x" + "2" * 40]
        jobs = [
            {
                "job_id": "single",
                "verification_mode": "signed_quorum",
                "eligible_verifiers": configured[:1],
                "threshold": 1,
            },
            {
                "job_id": "legacy",
                "verification_mode": "signed_quorum",
                "eligible_verifiers": configured,
                "threshold": 2,
            },
            {
                "job_id": "unsupported",
                "verification_mode": "signed_quorum",
                "eligible_verifiers": [configured[1]],
                "threshold": 1,
            },
        ]
        self.assertEqual(
            [job["job_id"] for job in pipeline.selected_jobs(jobs, configured, 5)],
            ["single", "legacy"],
        )

    def test_regression_job_rejects_zero_or_ambiguous_verifiers(self) -> None:
        base = {
            "verification_mode": "signed_quorum",
            "eligible_verifiers": ["0x" + "1" * 40],
            "threshold": 1,
        }
        self.assertEqual(pipeline.required_job_signers(base), base["eligible_verifiers"])
        for changed in (
            {**base, "threshold": 0},
            {**base, "threshold": 2},
            {
                **base,
                "threshold": 2,
                "eligible_verifiers": ["0x" + "1" * 40, "0x" + "1" * 40],
            },
        ):
            with self.subTest(changed=changed), self.assertRaises(pipeline.PipelineError):
                pipeline.required_job_signers(changed)

    def test_verifier_signing_uses_a_dedicated_rpc_configuration(self) -> None:
        workflow_root = SCRIPT.parent.parent / ".github" / "workflows"
        for name in (
            "regression-verifier-signing-reusable.yml",
            "regression-verifier-signer.yml",
        ):
            with self.subTest(workflow=name):
                workflow = (workflow_root / name).read_text(encoding="utf-8")
                self.assertIn("vars.REGRESSION_VERIFIER_RPC_URL", workflow)
                self.assertNotIn("vars.BASE_MAINNET_RPC_URL", workflow)

    def test_stale_runner_revision_skips_every_signing_job(self) -> None:
        workflow = (
            SCRIPT.parent.parent / ".github" / "workflows" / "regression-verifier-signer.yml"
        ).read_text(encoding="utf-8")
        self.assertIn('echo "authorized=false" >> "$GITHUB_OUTPUT"', workflow)
        self.assertIn('echo "authorized=true" >> "$GITHUB_OUTPUT"', workflow)
        self.assertEqual(
            workflow.count("if: needs.authorize-run.outputs.authorized == 'true'"),
            3,
        )
        self.assertNotIn("Refusing to sign a candidate from stale main", workflow)

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

    def test_runner_pulls_only_the_exact_committed_image(self) -> None:
        manifest = {
            "image": f"docker.io/library/python@sha256:{'a' * 64}",
            "platform": "linux/amd64",
        }
        with mock.patch.object(pipeline, "run", return_value="") as run:
            pipeline.pull_pinned_image(manifest, "docker")
        run.assert_called_once_with(
            [
                "docker",
                "pull",
                "--platform",
                "linux/amd64",
                manifest["image"],
            ]
        )
        for image in [
            "docker.io/library/python:3.12",
            f"DOCKER.IO/library/python@sha256:{'a' * 64}",
            f"docker.io/library/python@sha256:{'g' * 64}",
        ]:
            with self.subTest(image=image), self.assertRaises(pipeline.PipelineError):
                pipeline.pull_pinned_image(
                    {"image": image, "platform": "linux/amd64"},
                    "docker",
                )

    def test_one_infrastructure_failure_does_not_block_later_jobs(self) -> None:
        verifier = "0x" + "1" * 40
        jobs = [
            {
                "job_id": name,
                "verification_mode": "signed_quorum",
                "eligible_verifiers": [verifier],
                "threshold": 1,
                "bounty_contract": "0x" + digit * 40,
                "round": 1,
            }
            for name, digit in (("too-large", "2"), ("healthy", "3"))
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = SimpleNamespace(
                verifier=[verifier],
                api_base="https://example.test",
                network="base-mainnet",
                max_jobs=5,
                output=root / "output",
                worker=root / "worker",
                staging=root / "staging",
            )
            healthy_candidate = {
                "schema": pipeline.CANDIDATE_SCHEMA,
                "job": jobs[1],
                "outcome": {"verdict": "passed"},
                "runner_revision": "test",
            }
            with mock.patch.object(pipeline, "verification_jobs", return_value=jobs), mock.patch.object(
                pipeline,
                "run_job",
                side_effect=[
                    pipeline.PipelineError(
                        "GitHub archive exceeds the compressed input limit"
                    ),
                    healthy_candidate,
                ],
            ):
                pipeline.command_run(args)

            manifest = json.loads((args.output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual([item["job_id"] for item in manifest["candidates"]], ["healthy"])
            self.assertEqual([item["job_id"] for item in manifest["failures"]], ["too-large"])
            failure = json.loads(
                (args.output / manifest["failures"][0]["file"]).read_text(encoding="utf-8")
            )
            self.assertEqual(failure["schema"], pipeline.FAILURE_SCHEMA)
            self.assertEqual(failure["error_code"], "source_archive_too_large")
            self.assertFalse(failure["retryable"])
            self.assertFalse(failure["verdict_emitted"])

    def test_preflight_uses_the_same_job_runner_and_requires_a_pass(self) -> None:
        verifier = "0x" + "1" * 40
        job = {
            "job_id": "preflight-job",
            "verification_mode": "signed_quorum",
            "eligible_verifiers": [verifier],
            "threshold": 1,
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            input_path = root / "preparation.json"
            input_path.write_text(
                json.dumps({"verifier_preflight": {"job": job}}), encoding="utf-8"
            )
            args = SimpleNamespace(
                input=input_path,
                output=root / "receipt.json",
                worker=root / "worker",
                staging=root / "staging",
            )
            candidate = {
                "schema": pipeline.CANDIDATE_SCHEMA,
                "job": job,
                "outcome": {"verdict": "passed", "response_hash": "0x" + "a" * 64},
                "runner_revision": "test",
            }
            with mock.patch.object(pipeline, "run_job", return_value=candidate) as run_job:
                pipeline.command_preflight(args)
            run_job.assert_called_once()
            receipt = json.loads(args.output.read_text(encoding="utf-8"))
            self.assertEqual(receipt["schema"], pipeline.PREFLIGHT_SCHEMA)
            self.assertEqual(receipt["status"], "passed")
            self.assertTrue(receipt["safe_to_sign"])

            candidate["outcome"] = {"verdict": "failed", "response_hash": "0x" + "b" * 64}
            with mock.patch.object(pipeline, "run_job", return_value=candidate), self.assertRaises(
                pipeline.PipelineError
            ):
                pipeline.command_preflight(args)
            receipt = json.loads(args.output.read_text(encoding="utf-8"))
            self.assertEqual(receipt["status"], "benchmark_rejected")
            self.assertFalse(receipt["safe_to_sign"])

    def test_runner_artifact_is_uploaded_even_after_an_unexpected_step_failure(self) -> None:
        workflow = (
            SCRIPT.parent.parent / ".github" / "workflows" / "regression-verifier-runner.yml"
        ).read_text(encoding="utf-8")
        upload = workflow.split(
            "uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
            maxsplit=1,
        )[1]
        self.assertIn("if: always()", upload)


if __name__ == "__main__":
    unittest.main()
