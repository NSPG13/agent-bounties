from __future__ import annotations

import importlib.util
import io
import os
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
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
    def test_manifest_files_are_exact_content_addressed_basenames(self) -> None:
        job_id = "base-mainnet:test:1"
        candidate = pipeline.content_addressed_name("candidate", job_id)
        self.assertEqual(
            pipeline.manifest_file(
                Path("candidates"),
                candidate,
                pipeline.CANDIDATE_FILE,
                "candidate manifest file",
            ),
            Path("candidates") / candidate,
        )
        for unsafe in ("../candidate-" + "a" * 64 + ".json", "manifest.json", "C:\\secret"):
            with self.subTest(unsafe=unsafe), self.assertRaises(pipeline.PipelineError):
                pipeline.manifest_file(
                    Path("candidates"),
                    unsafe,
                    pipeline.CANDIDATE_FILE,
                    "candidate manifest file",
                )

    def test_signing_key_is_not_exposed_before_local_digest_validation(self) -> None:
        signer = "0x" + "1" * 40
        digest = "0x" + "2" * 64
        response_hash = "0x" + "3" * 64
        events: list[str] = []
        job = {
            "job_id": "base-mainnet:test:1",
            "verification_mode": "signed_quorum",
            "threshold": 1,
            "eligible_verifiers": [signer],
        }
        current = {
            **job,
            "verification_expires_at": 2_000_001_000,
            "bounty_contract": "0x" + "4" * 40,
        }

        def fake_digest(*_args, **_kwargs) -> str:
            events.append("digest")
            return digest

        def fake_run(command: list[str], *, env=None) -> str:
            self.assertNotIn("REGRESSION_VERIFIER_PRIVATE_KEY", env)
            self.assertIn("digest", events)
            if command[1:3] == ["wallet", "address"]:
                events.append("key-address")
                return signer
            if command[1:3] == ["wallet", "sign"]:
                events.append("key-sign")
                return "0x" + "5" * 130
            self.fail(f"unexpected command: {command}")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(
            os.environ,
            {"REGRESSION_VERIFIER_PRIVATE_KEY": "test-secret"},
            clear=False,
        ):
            root = Path(temporary)
            candidates = root / "candidates"
            output = root / "output"
            candidates.mkdir()
            candidate_file = pipeline.content_addressed_name("candidate", job["job_id"])
            pipeline.write_json(
                candidates / "manifest.json",
                {
                    "schema": pipeline.MANIFEST_SCHEMA,
                    "candidates": [{"job_id": job["job_id"], "file": candidate_file}],
                },
            )
            pipeline.write_json(
                candidates / candidate_file,
                {
                    "schema": pipeline.CANDIDATE_SCHEMA,
                    "job": job,
                    "outcome": {"verdict": "passed", "response_hash": response_hash},
                },
            )
            args = mock.Mock(
                private_key_env="REGRESSION_VERIFIER_PRIVATE_KEY",
                expected_signer=signer,
                candidates=candidates,
                output=output,
                api_base="https://api.agentbounties.app",
                network="base-mainnet",
                worker=Path("trusted-worker"),
                cast=Path("cast"),
                rpc_url="https://base-rpc.invalid",
            )
            with mock.patch.object(pipeline, "current_job", return_value=current), mock.patch.object(
                pipeline, "validate_candidate"
            ), mock.patch.object(
                pipeline, "attestation_digest", side_effect=fake_digest
            ), mock.patch.object(
                pipeline, "run", side_effect=fake_run
            ), mock.patch.object(
                pipeline.time, "time", return_value=2_000_000_000
            ):
                pipeline.command_sign(args)

        self.assertEqual(events, ["digest", "key-address", "key-sign"])

    def test_relay_key_is_not_exposed_before_exact_rpc_preflight(self) -> None:
        verifier = "0x" + "1" * 40
        keeper = "0x" + "2" * 40
        bounty = "0x" + "3" * 40
        response_hash = "0x" + "4" * 64
        signature = "0x" + "5" * 130
        job_id = "base-mainnet:test:relay"
        events: list[str] = []
        job = {
            "job_id": job_id,
            "verification_mode": "signed_quorum",
            "threshold": 1,
            "eligible_verifiers": [verifier],
            "bounty_contract": bounty,
        }

        def fake_preflight(*_args, **_kwargs) -> int:
            events.append("preflight")
            return 7

        def fake_immediate_nonce(*_args, **_kwargs) -> int:
            events.append("pre-send-nonce")
            return 7

        def fake_run(command: list[str], *, env=None) -> str:
            self.assertNotIn("BASE_KEEPER_PRIVATE_KEY", env)
            self.assertIn("preflight", events)
            if command[1:3] == ["wallet", "address"]:
                events.append("key-address")
                return keeper
            if command[1] == "send":
                events.append("send")
                self.assertIn("--chain", command)
                self.assertEqual(command[command.index("--chain") + 1], "8453")
                self.assertEqual(command[command.index("--nonce") + 1], "7")
                self.assertEqual(
                    command[command.index("--gas-limit") + 1],
                    str(pipeline.RELAY_GAS_LIMIT),
                )
                self.assertEqual(
                    command[command.index("--gas-price") + 1],
                    str(pipeline.RELAY_MAX_FEE_PER_GAS),
                )
                return '{"transactionHash":"0x' + "6" * 64 + '","status":"0x1"}'
            self.fail(f"unexpected command: {command}")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(
            os.environ,
            {"BASE_KEEPER_PRIVATE_KEY": "test-secret"},
            clear=False,
        ):
            root = Path(temporary)
            candidates = root / "candidates"
            attestations = root / "attestations"
            candidates.mkdir()
            attestations.mkdir()
            candidate_file = pipeline.content_addressed_name("candidate", job_id)
            attestation_file = pipeline.content_addressed_name("attestation", job_id)
            pipeline.write_json(
                candidates / "manifest.json",
                {
                    "schema": pipeline.MANIFEST_SCHEMA,
                    "candidates": [{"job_id": job_id, "file": candidate_file}],
                },
            )
            pipeline.write_json(
                candidates / candidate_file,
                {
                    "schema": pipeline.CANDIDATE_SCHEMA,
                    "job": job,
                    "outcome": {"verdict": "passed", "response_hash": response_hash},
                },
            )
            pipeline.write_json(
                attestations / "manifest.json",
                {
                    "schema": pipeline.ATTESTATION_SCHEMA,
                    "signer": verifier,
                    "attestations": [{"job_id": job_id, "file": attestation_file}],
                },
            )
            pipeline.write_json(
                attestations / attestation_file,
                {
                    "schema": pipeline.ATTESTATION_SCHEMA,
                    "job_id": job_id,
                    "bounty_contract": bounty,
                    "verifier": verifier,
                    "passed": True,
                    "response_hash": response_hash,
                    "deadline": 2_000_000_000,
                    "signature": signature,
                },
            )
            args = mock.Mock(
                keeper_key_env="BASE_KEEPER_PRIVATE_KEY",
                expected_keeper=keeper,
                verifier=[verifier],
                candidates=candidates,
                attestations=[attestations],
                api_base="https://api.agentbounties.app",
                network="base-mainnet",
                worker=Path("trusted-worker"),
                cast=Path("cast"),
                rpc_url="https://relay.invalid",
            )
            with mock.patch.object(pipeline, "current_job", return_value=job), mock.patch.object(
                pipeline, "validate_candidate"
            ), mock.patch.object(
                pipeline, "relay_rpc_preflight", side_effect=fake_preflight
            ), mock.patch.object(
                pipeline, "keeper_nonce_preflight", side_effect=fake_immediate_nonce
            ), mock.patch.object(
                pipeline, "run", side_effect=fake_run
            ):
                pipeline.command_relay(args)

        self.assertEqual(events, ["preflight", "key-address", "pre-send-nonce", "send"])

    def test_relay_rpc_preflight_rejects_unbounded_gas_before_key_use(self) -> None:
        commands: list[list[str]] = []

        def fake_run(command: list[str], *, env=None) -> str:
            self.assertNotIn("BASE_KEEPER_PRIVATE_KEY", env)
            commands.append(command)
            return {
                "chain-id": "8453",
                "call": "0x",
                "estimate": str(pipeline.RELAY_GAS_LIMIT + 1),
            }[command[1]]

        with mock.patch.object(pipeline, "run", side_effect=fake_run), mock.patch.object(
            pipeline, "canonical_bounty_rpc_preflight"
        ), self.assertRaisesRegex(pipeline.PipelineError, "gas estimate exceeds"):
            pipeline.relay_rpc_preflight(
                Path("cast"),
                "https://relay.invalid",
                "0x" + "1" * 40,
                "0x" + "2" * 40,
                "[]",
                {"PUBLIC_VALUE": "kept"},
            )
        self.assertEqual([command[1] for command in commands], ["chain-id", "call", "estimate"])

    def test_keeper_nonce_preflight_rejects_pending_keeper_nonce(self) -> None:
        commands: list[list[str]] = []

        def fake_run(command: list[str], *, env=None) -> str:
            self.assertNotIn("BASE_KEEPER_PRIVATE_KEY", env)
            commands.append(command)
            if command[1] == "chain-id":
                return "8453"
            if command[1] == "call":
                return "0x"
            if command[1] == "estimate":
                return "100000"
            if command[1] == "nonce":
                return "7" if command[command.index("--block") + 1] == "latest" else "8"
            self.fail(f"unexpected command: {command}")

        with mock.patch.object(pipeline, "run", side_effect=fake_run), self.assertRaisesRegex(
            pipeline.PipelineError, "another pending transaction"
        ):
            pipeline.keeper_nonce_preflight(
                Path("cast"),
                "https://relay.invalid",
                "0x" + "2" * 40,
                {"PUBLIC_VALUE": "kept"},
            )
        nonce_blocks = [
            command[command.index("--block") + 1]
            for command in commands
            if command[1] == "nonce"
        ]
        self.assertEqual(nonce_blocks, ["latest", "pending"])

    def test_keeper_nonce_preflight_requires_two_matching_rpcs(self) -> None:
        commands: list[list[str]] = []

        def fake_run(command: list[str], *, env=None) -> str:
            self.assertNotIn("BASE_KEEPER_PRIVATE_KEY", env)
            commands.append(command)
            return "8453" if command[1] == "chain-id" else "7"

        with mock.patch.object(pipeline, "run", side_effect=fake_run):
            nonce = pipeline.keeper_nonce_preflight(
                Path("cast"),
                "https://relay.invalid",
                "0x" + "2" * 40,
                {"PUBLIC_VALUE": "kept"},
            )
        self.assertEqual(nonce, 7)
        rpc_urls = {
            command[command.index("--rpc-url") + 1]
            for command in commands
            if "--rpc-url" in command
        }
        self.assertEqual(rpc_urls, {"https://relay.invalid", "https://mainnet.base.org"})

    def test_keeper_nonce_preflight_rejects_equal_but_stale_rpc_pair(self) -> None:
        def fake_run(command: list[str], *, env=None) -> str:
            if command[1] == "chain-id":
                return "8453"
            rpc_url = command[command.index("--rpc-url") + 1]
            return "7" if rpc_url == "https://relay.invalid" else "6"

        with mock.patch.object(pipeline, "run", side_effect=fake_run), self.assertRaisesRegex(
            pipeline.PipelineError, "independent nonce RPCs disagree"
        ):
            pipeline.keeper_nonce_preflight(
                Path("cast"),
                "https://relay.invalid",
                "0x" + "2" * 40,
                {"PUBLIC_VALUE": "kept"},
            )

    def test_canonical_bounty_preflight_rejects_wrong_nonempty_runtime(self) -> None:
        commands: list[list[str]] = []

        def fake_run(command: list[str], *, env=None) -> str:
            self.assertNotIn("BASE_KEEPER_PRIVATE_KEY", env)
            commands.append(command)
            return "0x60016000"

        with mock.patch.object(pipeline, "run", side_effect=fake_run), self.assertRaisesRegex(
            pipeline.PipelineError, "not the precommitted canonical clone"
        ):
            pipeline.canonical_bounty_rpc_preflight(
                Path("cast"),
                "https://relay.invalid",
                "0x" + "1" * 40,
                {"PUBLIC_VALUE": "kept"},
            )
        self.assertEqual(len(commands), 1)
        self.assertEqual(commands[0][1], "code")
        self.assertEqual(commands[0][commands[0].index("--block") + 1], "safe")

    def test_canonical_bounty_preflight_binds_factory_and_token(self) -> None:
        commands: list[list[str]] = []

        def fake_run(command: list[str], *, env=None) -> str:
            commands.append(command)
            if command[1] == "code":
                return pipeline.CANONICAL_BOUNTY_RUNTIME
            if command[-1] == "factory()(address)":
                return pipeline.CANONICAL_BOUNTY_FACTORY
            if command[-1] == "settlementToken()(address)":
                return pipeline.CANONICAL_SETTLEMENT_TOKEN
            self.fail(f"unexpected command: {command}")

        with mock.patch.object(pipeline, "run", side_effect=fake_run):
            pipeline.canonical_bounty_rpc_preflight(
                Path("cast"),
                "https://relay.invalid",
                "0x" + "1" * 40,
                {"PUBLIC_VALUE": "kept"},
            )
        self.assertEqual([command[1] for command in commands], ["code", "call", "call"])
        self.assertTrue(
            all(command[command.index("--block") + 1] == "safe" for command in commands)
        )

    def test_canonical_bounty_preflight_rejects_wrong_provenance(self) -> None:
        cases = (
            ("factory()(address)", "0x" + "3" * 40, "canonical factory"),
            ("settlementToken()(address)", "0x" + "4" * 40, "canonical Base USDC"),
        )
        for wrong_field, wrong_value, expected_error in cases:
            with self.subTest(field=wrong_field):
                def fake_run(command: list[str], *, env=None) -> str:
                    if command[1] == "code":
                        return pipeline.CANONICAL_BOUNTY_RUNTIME
                    if command[-1] == "factory()(address)":
                        return (
                            wrong_value
                            if wrong_field == "factory()(address)"
                            else pipeline.CANONICAL_BOUNTY_FACTORY
                        )
                    if command[-1] == "settlementToken()(address)":
                        return wrong_value
                    self.fail(f"unexpected command: {command}")

                with mock.patch.object(pipeline, "run", side_effect=fake_run), self.assertRaisesRegex(
                    pipeline.PipelineError, expected_error
                ):
                    pipeline.canonical_bounty_rpc_preflight(
                        Path("cast"),
                        "https://relay.invalid",
                        "0x" + "1" * 40,
                        {"PUBLIC_VALUE": "kept"},
                    )

    def test_candidate_validation_strips_signing_secrets_from_worker(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(
            os.environ,
            {"REGRESSION_VERIFIER_PRIVATE_KEY": "test-secret", "PUBLIC_VALUE": "kept"},
            clear=False,
        ), mock.patch.object(pipeline, "run", return_value="ok") as invoked:
            pipeline.validate_candidate(
                Path("trusted-worker"),
                {"schema": pipeline.CANDIDATE_SCHEMA, "job": {}},
                {},
                Path(temporary),
                secret_names=("REGRESSION_VERIFIER_PRIVATE_KEY",),
            )
        environment = invoked.call_args.kwargs["env"]
        self.assertNotIn("REGRESSION_VERIFIER_PRIVATE_KEY", environment)
        self.assertEqual(environment["PUBLIC_VALUE"], "kept")

    def test_rpc_digest_must_equal_local_eip712_digest(self) -> None:
        good = "0x" + "1" * 64
        bad = "0x" + "2" * 64
        current = {"bounty_contract": "0x" + "3" * 40}

        def fake_run(command: list[str], *, env=None) -> str:
            self.assertNotIn("REGRESSION_VERIFIER_PRIVATE_KEY", env)
            if command[1] == "chain-id":
                return "8453"
            if command[1] == "code":
                return "0x60016000"
            if command[1] == "call":
                return bad
            self.fail(f"unexpected command: {command}")

        with mock.patch.object(pipeline, "run", side_effect=fake_run), mock.patch.object(
            pipeline, "canonical_bounty_rpc_preflight"
        ), mock.patch.object(pipeline, "local_attestation_digest", return_value=good):
            with self.assertRaisesRegex(
                pipeline.PipelineError,
                "differs from the local EIP-712 digest",
            ):
                pipeline.attestation_digest(
                    Path("cast"),
                    "https://secondary.invalid",
                    current,
                    "0x" + "4" * 40,
                    True,
                    "0x" + "5" * 64,
                    2_000_000_000,
                    {"PUBLIC_VALUE": "kept"},
                )

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

    def test_subdirectory_limits_ignore_unrelated_archive_entries(self) -> None:
        value = archive(
            [
                (f"repo-root/unrelated/{index}.txt", b"ignored", "file")
                for index in range(200)
            ]
            + [("repo-root/bench/test.txt", b"expected", "file")]
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pipeline.extract_snapshot(
                value,
                root,
                subdirectory="bench",
                max_bytes=100,
                max_files=1,
            )
            self.assertEqual((root / "test.txt").read_bytes(), b"expected")
            self.assertFalse((root / "unrelated").exists())

    def test_subdirectory_entry_guard_applies_after_selection(self) -> None:
        value = archive(
            [(f"repo-root/bench/{index}", None, "dir") for index in range(105)]
        )
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(
                pipeline.PipelineError,
                "selected snapshot has too many entries",
            ):
                pipeline.extract_snapshot(
                    value,
                    Path(temporary),
                    subdirectory="bench",
                    max_bytes=100,
                    max_files=1,
                )

    def test_archive_transport_caps_are_independent_of_snapshot_limits(self) -> None:
        self.assertEqual(pipeline.MAX_GITHUB_SOURCE_ARCHIVE_BYTES, 256 * 1024 * 1024)
        self.assertEqual(pipeline.MAX_GITHUB_BENCHMARK_ARCHIVE_BYTES, 128 * 1024 * 1024)
        self.assertEqual(
            pipeline.MAX_GITHUB_SOURCE_ARCHIVE_UNCOMPRESSED_BYTES,
            512 * 1024 * 1024,
        )
        self.assertEqual(
            pipeline.MAX_GITHUB_BENCHMARK_ARCHIVE_UNCOMPRESSED_BYTES,
            256 * 1024 * 1024,
        )

    def test_unselected_member_cannot_exceed_whole_archive_budget(self) -> None:
        value = archive(
            [
                ("repo-root/unrelated/large.bin", b"0123456789", "file"),
                ("repo-root/bench/test.txt", b"expected", "file"),
            ]
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaisesRegex(
                pipeline.PipelineError,
                "archive exceeds the uncompressed input limit",
            ):
                pipeline.extract_snapshot(
                    value,
                    root,
                    subdirectory="bench",
                    max_bytes=100,
                    max_files=1,
                    max_archive_bytes=9,
                )
            self.assertFalse((root / "test.txt").exists())

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

    def test_glama_lifecycle_benchmark_is_not_signable_without_canonical_reconciliation(self) -> None:
        for repository in ("NSPG13/agent-bounties", "nspg13/AGENT-BOUNTIES"):
            job = {
                "terms": {
                    "document": {
                        "benchmark": {
                            "source": {
                                "kind": "github_commit",
                                "repository": repository,
                                "commit": "b" * 40,
                                "subdirectory": "benchmarks/distribution-v1/glama-onboarding-audit",
                            }
                        }
                    }
                }
            }
            with self.assertRaisesRegex(pipeline.PipelineError, "independently reconciled"):
                pipeline.reject_unreconciled_canonical_lifecycle_benchmark(job)

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


if __name__ == "__main__":
    unittest.main()
