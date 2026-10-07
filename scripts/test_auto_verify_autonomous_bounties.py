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
                terms={"policy_hash": POLICY_HASH, "document": {"benchmark": {"engine": auto.ENGINE, "difficulty_bits": 24}}}
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

    def test_other_factories_and_modules_are_skipped(self) -> None:
        for state, reason in (
            (self.submitted(factory="0x" + "55" * 20), "autonomous-v1 factory"),
            (self.submitted(verifier_module=relay.STANDING_META_V2_VERIFIER_MODULE), "leading-zero-work"),
        ):
            result, relay_envelope = self.settle_one(state)
            self.assertEqual(result["outcome"], "skipped")
            self.assertIn(reason, result["reason"])
            relay_envelope.assert_not_called()

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
