import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import auto_verify_autonomous_bounties as auto
import relay_autonomous_action as relay
from test_relay_autonomous_action import CONTRACT, NOW, SOLVER, bounty_state


BOUNTY_ID = "0x" + "ab" * 32
POLICY_HASH = "0x" + "44" * 32


def job(**overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "job_id": "job",
        "network": "base-mainnet",
        "bounty_id": BOUNTY_ID,
        "bounty_contract": CONTRACT,
        "round": 3,
        "solver_wallet": SOLVER,
        "verification_mode": "deterministic_module",
        "verifier_module": auto.MODULE,
        "eligible_verifiers": [],
        "threshold": 1,
        "verification_expires_at": NOW + 3_600,
        "terms": {
            "policy_hash": POLICY_HASH,
            "document": {
                "benchmark": {"engine": auto.ENGINE, "difficulty_bits": 16},
                "contract_terms": {"protocol_version": "agent-bounties/autonomous-v1"},
            },
        },
        "submission_evidence": {
            "artifact_hash": "0x" + "11" * 32,
            "evidence_hash": "0x" + "22" * 32,
        },
    }
    value.update(overrides)
    return value


def zero_hash(_: bytes) -> bytes:
    return bytes(32)


class MiningTests(unittest.TestCase):
    def test_mined_nonce_matches_the_deployed_module(self) -> None:
        # Vector checked with `cast call` against LeadingZeroWorkVerifier
        # 0xcc6059ce…579e on Base mainnet: verify(...) returned
        # (true, 0x0000f9d9…6a2f) for nonce 101762 and false for 101763.
        from _shared.evm import keccak_bytes

        prefix = auto.work_prefix(job())
        nonce = auto.mine_nonce(prefix)
        self.assertEqual(nonce, 101_762)
        self.assertEqual(
            keccak_bytes(prefix + nonce.to_bytes(32, "big")).hex(),
            "0000f9d986fa38592a9ab5866d20e52507df12b2908d2b1851f87aa32dd26a2f",
        )

    def test_pinned_clone_codehashes_are_the_eip1167_runtimes_of_their_implementations(self) -> None:
        # The relay tests run before pycryptodome is installed, so the derivation lives here.
        from _shared.evm import keccak256

        for implementation, codehash in (
            (relay.IMPLEMENTATION, relay.CLONE_CODEHASH),
            (relay.V2_IMPLEMENTATION, relay.V2_CLONE_CODEHASH),
        ):
            runtime = bytes.fromhex(
                "363d3d373d3d3d363d73" + implementation[2:] + "5af43d82803e903d91602b57fd5bf3"
            )
            self.assertEqual(keccak256(runtime), codehash)

    def test_work_prefix_is_the_abi_encoding_of_the_committed_values(self) -> None:
        prefix = auto.work_prefix(job())
        self.assertEqual(len(prefix), 6 * 32)
        self.assertEqual(prefix[:32], bytes.fromhex("ab" * 32))
        self.assertEqual(prefix[32:64], (3).to_bytes(32, "big"))
        self.assertEqual(prefix[64:96], bytes.fromhex(SOLVER[2:]).rjust(32, b"\0"))
        self.assertEqual(prefix[160:], bytes.fromhex("44" * 32))

    def test_mining_is_bounded(self) -> None:
        with self.assertRaisesRegex(auto.AutoVerifyError, "within 8 attempts"):
            auto.mine_nonce(b"", max_attempts=8, hash_fn=lambda _: b"\xff" * 32)

    def test_malformed_committed_values_are_refused(self) -> None:
        for broken in (
            job(bounty_id="0x1234"),
            job(solver_wallet="not-an-address"),
            job(round=1 << 64),
            job(round=True),
        ):
            with self.assertRaises(auto.AutoVerifyError):
                auto.work_prefix(broken)


class DiscoverTests(unittest.TestCase):
    def test_only_submitted_leading_zero_jobs_become_envelopes(self) -> None:
        signed = job(verification_mode="signed_quorum", verifier_module=None)
        routed = job(verifier_module="0x380c1af7d5bf4b1bd9a3aeff95bde9a0e7a46731")
        envelopes, skipped = auto.discover([signed, routed, job()], now=NOW, hash_fn=zero_hash)
        self.assertEqual(
            envelopes,
            [{"bounty_contract": CONTRACT, "round": 3, "proof": "0x" + "00" * 32}],
        )
        # Jobs for other verifiers are not this keeper's business and are not reported.
        self.assertEqual(skipped, [])

    def test_unsafe_or_unsupported_jobs_are_skipped_with_a_reason(self) -> None:
        cases = {
            "network is not base-mainnet": job(network="base-sepolia"),
            "deterministic verification requires threshold 1": job(threshold=2),
            "benchmark is not the 16-bit leading-zero-work engine": job(
                terms={
                    "policy_hash": POLICY_HASH,
                    "document": {
                        "benchmark": {"engine": auto.ENGINE, "difficulty_bits": 24},
                        "contract_terms": {"protocol_version": "agent-bounties/autonomous-v1"},
                    },
                }
            ),
            "not an autonomous-v1 or autonomous-v2 bounty": job(
                terms={
                    "policy_hash": POLICY_HASH,
                    "document": {
                        "benchmark": {"engine": auto.ENGINE, "difficulty_bits": 16},
                        "contract_terms": {"protocol_version": "agent-bounties/autonomous-v3"},
                    },
                }
            ),
            "verification window closes too soon": job(
                verification_expires_at=NOW + auto.MIN_REMAINING_SECONDS
            ),
            "round is invalid": job(round=0),
            "bounty contract is invalid": job(bounty_contract="0x1234"),
            "terms are missing a benchmark": job(terms={"policy_hash": POLICY_HASH}),
        }
        for reason, item in cases.items():
            envelopes, skipped = auto.discover([item], now=NOW, hash_fn=zero_hash)
            self.assertEqual(envelopes, [], reason)
            self.assertEqual(skipped[0]["reason"], reason)
        envelopes, skipped = auto.discover(["not a job"], now=NOW, hash_fn=zero_hash)
        self.assertEqual((envelopes, skipped[0]["reason"]), ([], "malformed job"))

    def test_most_urgent_jobs_go_first_and_the_rest_wait(self) -> None:
        jobs = [
            job(
                bounty_contract=f"0x{index:040x}",
                verification_expires_at=NOW + 10_000 - index,
            )
            for index in range(1, 8)
        ]
        envelopes, skipped = auto.discover(jobs, now=NOW, max_jobs=5, hash_fn=zero_hash)
        self.assertEqual(
            [item["bounty_contract"] for item in envelopes],
            [f"0x{index:040x}" for index in (7, 6, 5, 4, 3)],
        )
        self.assertEqual(
            [(item["bounty_contract"], item["reason"]) for item in skipped],
            [(f"0x{index:040x}", "deferred to the next run") for index in (2, 1)],
        )

    def test_v1_and_v2_jobs_are_both_candidates(self) -> None:
        v2_terms = {
            "policy_hash": POLICY_HASH,
            "document": {
                "benchmark": {"engine": auto.ENGINE, "difficulty_bits": 16},
                "contract_terms": {"protocol_version": "agent-bounties/autonomous-v2"},
            },
        }
        v2_contract = "0x" + "06" * 20
        jobs = [
            job(bounty_contract=v2_contract, verification_expires_at=NOW + 1_000, terms=v2_terms),
            job(verification_expires_at=NOW + 3_000),
        ]
        envelopes, skipped = auto.discover(jobs, now=NOW, hash_fn=zero_hash)
        self.assertEqual([item["bounty_contract"] for item in envelopes], [v2_contract, CONTRACT])
        self.assertEqual(skipped, [])

    def test_duplicate_contracts_are_settled_once(self) -> None:
        envelopes, skipped = auto.discover([job(), job(round=4)], now=NOW, hash_fn=zero_hash)
        self.assertEqual(len(envelopes), 1)
        self.assertEqual(skipped[0]["reason"], "duplicate bounty contract")

    def test_mining_failure_skips_only_that_job(self) -> None:
        envelopes, skipped = auto.discover(
            [job(bounty_id="0x12")], now=NOW, hash_fn=zero_hash
        )
        self.assertEqual(envelopes, [])
        self.assertIn("32-byte", skipped[0]["reason"])


RECENT = "2027-01-15T08:00:00Z"
RECENT_NOW = 1_800_000_000.0  # 2027-01-15T08:00:00Z
RELAY_WORKFLOW = ".github/workflows/autonomous-gas-relay.yml"
AUTO_WORKFLOW = ".github/workflows/autonomous-auto-verify.yml"


def run_pages(*pages: list[dict[str, object]], total: int | None = None):
    """Fake GitHub listing: the same pages for every status, with a fixed total."""

    count = sum(len(page) for page in pages) if total is None else total
    calls: list[str] = []

    def fetch(url: str, token: str) -> dict[str, object]:
        calls.append(url)
        page = int(url.rsplit("page=", 1)[1])
        runs = pages[page - 1] if page <= len(pages) else []
        status = url.split("status=", 1)[1].split("&", 1)[0]
        return {"total_count": count, "workflow_runs": runs if status == "in_progress" else []}

    return fetch, calls


class KeeperLockTests(unittest.TestCase):
    def test_lock_workflows_are_read_from_the_repository(self) -> None:
        # The default root comes from the script's location, not the working directory.
        workflows = auto.keeper_lock_workflows()
        self.assertIn(RELAY_WORKFLOW, workflows)
        self.assertIn(AUTO_WORKFLOW, workflows)
        self.assertNotIn(".github/workflows/ci.yml", workflows)

    def test_missing_lock_workflows_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(auto.AutoVerifyError, "no keeper-lock workflow"):
                auto.keeper_lock_workflows(pathlib.Path(directory))

    def test_other_active_keeper_runs_block_the_hand_off(self) -> None:
        fetch, _ = run_pages(
            [
                {"id": 7, "path": AUTO_WORKFLOW, "created_at": RECENT},
                {"id": 8, "path": ".github/workflows/ci.yml", "created_at": RECENT},
                {"id": 9, "path": RELAY_WORKFLOW, "created_at": RECENT},
                # Queued for more than a day: a phantom that holds no lock.
                {"id": 10, "path": RELAY_WORKFLOW, "created_at": "2027-01-14T07:59:59Z"},
            ]
        )
        busy = auto.busy_keeper_runs(
            "NSPG13/agent-bounties",
            "token",
            7,
            {RELAY_WORKFLOW, AUTO_WORKFLOW},
            now=RECENT_NOW,
            fetch=fetch,
        )
        # The current run, non-keeper workflows and phantoms do not count.
        self.assertEqual(busy, [{"id": 9, "path": RELAY_WORKFLOW, "status": "in_progress"}])

    def test_every_active_status_is_inspected(self) -> None:
        fetch, calls = run_pages([])
        self.assertEqual(
            auto.busy_keeper_runs("o/r", "t", 1, {RELAY_WORKFLOW}, now=RECENT_NOW, fetch=fetch), []
        )
        self.assertEqual(
            [url.split("status=", 1)[1].split("&", 1)[0] for url in calls],
            list(auto.ACTIVE_RUN_STATUSES),
        )

    def test_listing_is_paginated_and_fails_closed(self) -> None:
        page = [
            {"id": index, "path": ".github/workflows/ci.yml", "created_at": RECENT}
            for index in range(100)
        ]
        fetch, _ = run_pages(page, [{"id": 500, "path": RELAY_WORKFLOW, "created_at": RECENT}])
        check = lambda fetch: auto.busy_keeper_runs(  # noqa: E731
            "o/r", "t", 1, {RELAY_WORKFLOW}, now=RECENT_NOW, fetch=fetch
        )
        self.assertEqual([item["id"] for item in check(fetch)], [500])
        endless, _ = run_pages(*([page] * 6), total=10_000)
        with self.assertRaisesRegex(auto.AutoVerifyError, "too many active"):
            check(endless)
        with self.assertRaisesRegex(auto.AutoVerifyError, "malformed"):
            check(lambda *_: {"workflow_runs": []})
        undated, _ = run_pages([{"id": 3, "path": RELAY_WORKFLOW, "created_at": "soon"}])
        with self.assertRaisesRegex(auto.AutoVerifyError, "malformed"):
            check(undated)


class EnvelopeTests(unittest.TestCase):
    def test_discover_output_round_trips_into_relay_envelopes(self) -> None:
        envelopes, _ = auto.discover([job()], now=NOW, hash_fn=zero_hash)
        parsed = auto.parse_envelopes(json.dumps(envelopes))
        self.assertEqual(
            parsed,
            [
                {
                    "schema": relay.SCHEMA,
                    "action": "settle",
                    "network": "base-mainnet",
                    "bounty_contract": CONTRACT,
                    "round": 3,
                    "proof": "0x" + "00" * 32,
                }
            ],
        )

    def test_envelopes_are_strictly_validated(self) -> None:
        good = {"bounty_contract": CONTRACT, "round": 3, "proof": "0x" + "00" * 32}
        bad_inputs = [
            "{}",
            json.dumps([dict(good, action="claim")]),
            json.dumps([dict(good, proof="0x00")]),
            json.dumps([dict(good, round=0)]),
            json.dumps([good, good]),
            json.dumps([dict(good, bounty_contract=f"0x{i:040x}") for i in range(1, 7)]),
            json.dumps(["x"]),
            "[" + " " * auto.MAX_ENVELOPES_BYTES + "]",
        ]
        for text in bad_inputs:
            with self.assertRaises((auto.AutoVerifyError, relay.RelayError), msg=text[:60]):
                auto.parse_envelopes(text)


class SettleTests(unittest.TestCase):
    envelope = {
        "schema": relay.SCHEMA,
        "action": "settle",
        "network": "base-mainnet",
        "bounty_contract": CONTRACT,
        "round": 1,
        "proof": "0x" + "99" * 32,
    }

    def submitted(self, **overrides: object) -> relay.BountyState:
        values: dict[str, object] = {
            "status": relay.STATUS_SUBMITTED,
            "round": 1,
            "solver": SOLVER,
            "verification_expires_at": NOW + 3_600,
        }
        values.update(overrides)
        return bounty_state(**values)

    def settle_one(self, state: relay.BountyState, relayed: dict[str, object] | None = None):
        with mock.patch.object(relay, "read_state", return_value=state), mock.patch.object(
            relay, "relay_envelope", return_value=relayed or {"outcome": "relayed"}
        ) as relay_envelope:
            result = auto.settle_one(
                object(), self.envelope, execute=True, private_key="0x" + "01" * 32
            )
        return result, relay_envelope

    def test_passing_submission_is_relayed_without_waiting(self) -> None:
        result, relay_envelope = self.settle_one(self.submitted())
        self.assertEqual(result, {"outcome": "relayed"})
        _, kwargs = relay_envelope.call_args
        self.assertEqual(kwargs["source"], {"trigger": "auto_verify"})
        self.assertEqual(kwargs["state_wait_seconds"], 0)
        self.assertTrue(kwargs["execute"])

    def test_other_modules_are_skipped_and_unknown_factories_fail(self) -> None:
        result, relay_envelope = self.settle_one(
            self.submitted(verifier_module=relay.STANDING_META_V2_VERIFIER_MODULE)
        )
        self.assertEqual(result["outcome"], "skipped")
        self.assertIn("leading-zero-work", result["reason"])
        relay_envelope.assert_not_called()
        refused = relay.RelayError("bounty factory is not a recorded autonomous factory: 0x55")
        with mock.patch.object(relay, "read_state", side_effect=refused), mock.patch.object(
            relay, "relay_envelope"
        ) as relay_envelope:
            results = auto.settle(object(), [self.envelope], execute=True, private_key=None)
        self.assertEqual(results[0]["outcome"], "failed")
        relay_envelope.assert_not_called()

    def test_v2_submission_is_relayed_like_v1(self) -> None:
        from test_relay_autonomous_action import v2_state

        state = v2_state(
            status=relay.STATUS_SUBMITTED, round=1, solver=SOLVER, verification_expires_at=NOW + 3_600
        )
        result, relay_envelope = self.settle_one(state)
        self.assertEqual(result, {"outcome": "relayed"})
        relay_envelope.assert_called_once()

    def test_changed_lifecycle_is_skipped_or_already_applied(self) -> None:
        result, relay_envelope = self.settle_one(self.submitted(status=relay.STATUS_SETTLED))
        self.assertEqual(result["outcome"], "already_applied")
        for state in (
            self.submitted(status=relay.STATUS_CLAIMABLE),
            self.submitted(round=2),
        ):
            result, relay_envelope = self.settle_one(state)
            self.assertEqual(result["outcome"], "skipped")
            self.assertIn("no longer awaiting verification", result["reason"])
            relay_envelope.assert_not_called()

    def test_failing_proof_is_never_relayed(self) -> None:
        # The bounded relay itself pre-calls the module; a fail must not send.
        client = mock.Mock()
        client.call.return_value = "false 0x" + "00" * 32
        state = self.submitted(block_timestamp=NOW)
        with self.assertRaisesRegex(relay.RelayError, "did not return pass"):
            relay.action_call(client, self.envelope, state)

    def test_retryable_errors_stop_the_run_and_others_continue(self) -> None:
        second = dict(self.envelope, bounty_contract="0x" + "05" * 20)
        third = dict(self.envelope, bounty_contract="0x" + "06" * 20)
        outcomes = iter(
            [
                relay.RelayError("bounty moved"),
                relay.RelayError("keeper balance low", code="keeper_balance_low", retryable=True),
                {"outcome": "relayed"},
            ]
        )

        def fake(*_: object, **__: object) -> dict[str, object]:
            value = next(outcomes)
            if isinstance(value, Exception):
                raise value
            return value

        with mock.patch.object(auto, "settle_one", side_effect=fake):
            results = auto.settle(object(), [self.envelope, second, third], execute=True, private_key=None)
        self.assertEqual([item["outcome"] for item in results], ["failed", "retryable"])
        self.assertEqual(results[1]["error_code"], "keeper_balance_low")


class MainTests(unittest.TestCase):
    def test_discover_writes_github_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            output = root / "github_output"
            with mock.patch.object(auto, "fetch_jobs", return_value=[job()]), mock.patch.object(
                auto.time, "time", return_value=NOW
            ), mock.patch.object(auto, "mine_nonce", return_value=7):
                code = auto.main(
                    ["discover", "--github-output", str(output), "--report", str(root / "d.json")]
                )
            self.assertEqual(code, 0)
            lines = output.read_text().splitlines()
            self.assertEqual(lines[0], "count=1")
            envelopes = json.loads(lines[1].removeprefix("envelopes="))
            self.assertEqual(envelopes[0]["proof"], "0x" + (7).to_bytes(32, "big").hex())
            self.assertEqual(auto.parse_envelopes(json.dumps(envelopes))[0]["round"], 3)

    def discover_with_lock(self, environ: dict[str, str], busy: list[dict[str, object]]):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            output = root / "github_output"
            with mock.patch.object(auto, "fetch_jobs", return_value=[job()]), mock.patch.object(
                auto.time, "time", return_value=NOW
            ), mock.patch.object(auto, "mine_nonce", return_value=7), mock.patch.object(
                auto, "busy_keeper_runs", return_value=busy
            ) as checked, mock.patch.dict(auto.os.environ, environ, clear=True):
                code = auto.main(
                    [
                        "discover",
                        "--github-output",
                        str(output),
                        "--report",
                        str(root / "d.json"),
                        "--require-idle-keeper-lock",
                    ]
                )
            lines = output.read_text().splitlines() if output.exists() else []
            report = json.loads((root / "d.json").read_text())
        return code, lines, report, checked

    def test_busy_keeper_lock_defers_all_work(self) -> None:
        environ = {"GH_TOKEN": "t", "GITHUB_REPOSITORY": "o/r", "GITHUB_RUN_ID": "7"}
        busy = [{"id": 9, "path": RELAY_WORKFLOW, "status": "pending"}]
        code, lines, report, checked = self.discover_with_lock(environ, busy)
        self.assertEqual(code, 0)
        self.assertEqual(lines, ["count=0", "envelopes=[]"])
        self.assertEqual(report["outcome"], "deferred_keeper_lock_busy")
        self.assertEqual(checked.call_args.args[:3], ("o/r", "t", 7))
        self.assertIn(AUTO_WORKFLOW, checked.call_args.args[3])
        code, lines, report, _ = self.discover_with_lock(environ, [])
        self.assertEqual((code, lines[0], report["outcome"]), (0, "count=1", "discovered"))

    def test_lock_check_without_a_token_fails_closed(self) -> None:
        code, lines, report, checked = self.discover_with_lock({}, [])
        self.assertEqual((code, lines, report["outcome"]), (1, [], "failed"))
        checked.assert_not_called()

    def test_settle_fails_the_run_unless_every_bounty_is_handled(self) -> None:
        for results, expected in (
            ([{"outcome": "relayed"}, {"outcome": "skipped"}], 0),
            ([{"outcome": "relayed"}, {"outcome": "failed"}], 1),
        ):
            with tempfile.TemporaryDirectory() as directory, mock.patch.dict(
                auto.os.environ, {"AUTO_VERIFY_ENVELOPES": "[]"}
            ), mock.patch.object(auto, "settle", return_value=results):
                report = pathlib.Path(directory) / "s.json"
                code = auto.main(["settle", "--execute", "--report", str(report)])
                self.assertEqual(code, expected)
                self.assertEqual(json.loads(report.read_text())["results"], results)

    def test_settle_refuses_malformed_envelopes(self) -> None:
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(
            auto.os.environ, {"AUTO_VERIFY_ENVELOPES": '[{"bounty_contract": "0x1"}]'}
        ):
            report = pathlib.Path(directory) / "s.json"
            self.assertEqual(auto.main(["settle", "--report", str(report)]), 1)
            self.assertEqual(json.loads(report.read_text())["outcome"], "failed")


if __name__ == "__main__":
    unittest.main()
