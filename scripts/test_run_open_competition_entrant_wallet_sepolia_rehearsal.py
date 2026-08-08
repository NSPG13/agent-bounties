import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "run_open_competition_entrant_wallet_sepolia_rehearsal.py"
SPEC = importlib.util.spec_from_file_location("entrant_rehearsal", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class EntrantWalletRehearsalPreparationTests(unittest.TestCase):
    def test_prove_funding_call_batch_accepts_exact_admin_authorized_calls(self):
        admin = "0x" + "11" * 20
        keeper = "0x" + "22" * 20
        token = "0x" + "33" * 20
        transaction = {"from": "0x" + "44" * 20}
        trace = {
            "calls": [
                {
                    "from": transaction["from"],
                    "to": admin,
                    "input": "0x1626ba7e" + "00" * 64,
                    "output": "0x1626ba7e" + "00" * 28,
                },
                {"from": admin, "to": keeper, "input": "0x", "value": hex(500_000)},
                {
                    "from": admin,
                    "to": token,
                    "input": MODULE.erc20_transfer_calldata(keeper, 400_000),
                    "value": "0x0",
                },
            ]
        }

        proof = MODULE.prove_funding_call_batch(
            transaction, trace, admin, keeper, 500_000, token, 400_000
        )

        self.assertEqual(proof["admin_authorization"], "successful_eip1271_trace")
        self.assertEqual(proof["execution_sender"], admin)
        self.assertTrue(proof["exact_native_transfer"])
        self.assertTrue(proof["exact_token_transfer"])

    def test_prove_funding_call_batch_rejects_recipient_substitution(self):
        admin = "0x" + "11" * 20
        keeper = "0x" + "22" * 20
        token = "0x" + "33" * 20
        transaction = {"from": admin}
        trace = {
            "calls": [
                {"from": admin, "to": "0x" + "55" * 20, "input": "0x", "value": hex(500_000)},
                {
                    "from": admin,
                    "to": token,
                    "input": MODULE.erc20_transfer_calldata(keeper, 400_000),
                    "value": "0x0",
                },
            ]
        }

        with self.assertRaisesRegex(MODULE.RehearsalError, "native transfer"):
            MODULE.prove_funding_call_batch(
                transaction, trace, admin, keeper, 500_000, token, 400_000
            )

    def test_recovery_commitment_salt_is_deterministic_and_domain_separated(self):
        recovery_salt = "0x" + "99" * 32
        bounty = "0x" + "11" * 20
        solver = "0x" + "22" * 20

        first = MODULE.recovery_commitment_salt(recovery_salt, bounty, solver, "scenario:a")
        repeated = MODULE.recovery_commitment_salt(recovery_salt, bounty, solver, "scenario:a")
        other_tag = MODULE.recovery_commitment_salt(recovery_salt, bounty, solver, "scenario:b")
        other_solver = MODULE.recovery_commitment_salt(
            recovery_salt, bounty, "0x" + "33" * 20, "scenario:a"
        )

        self.assertEqual(first, repeated)
        self.assertEqual(len(first), 32)
        self.assertNotEqual(first, other_tag)
        self.assertNotEqual(first, other_solver)

    def test_prove_deployment_call_accepts_exact_relay_with_admin_eip1271(self):
        transaction = {
            "from": "0x" + "44" * 20,
            "to": "0x" + "55" * 20,
            "input": b"\x12\x34",
            "value": 0,
        }
        deployer = "0x" + "11" * 20
        admin = "0x" + "22" * 20
        calldata = "0xabcdef"
        trace = {
            "from": transaction["from"],
            "to": transaction["to"],
            "calls": [
                {
                    "from": transaction["to"],
                    "to": admin,
                    "input": "0x1626ba7e" + "00" * 64,
                    "output": "0x1626ba7e" + "00" * 28,
                },
                {
                    "from": transaction["to"],
                    "to": deployer,
                    "input": calldata,
                    "value": "0x0",
                },
            ],
        }

        proof = MODULE.prove_deployment_call(transaction, trace, deployer, calldata, admin)

        self.assertEqual(proof["submission_mode"], "metamask_relayed_transaction")
        self.assertEqual(proof["admin_authorization"], "successful_eip1271_trace")
        self.assertTrue(proof["exact_zero_value_deployer_call"])

    def test_prove_deployment_call_rejects_relay_without_admin_authorization(self):
        transaction = {
            "from": "0x" + "44" * 20,
            "to": "0x" + "55" * 20,
            "input": b"\x12\x34",
            "value": 0,
        }
        deployer = "0x" + "11" * 20
        admin = "0x" + "22" * 20
        trace = {
            "calls": [
                {
                    "from": transaction["to"],
                    "to": deployer,
                    "input": "0xabcdef",
                    "value": "0x0",
                }
            ]
        }

        with self.assertRaisesRegex(MODULE.RehearsalError, "admin EIP-1271"):
            MODULE.prove_deployment_call(transaction, trace, deployer, "0xabcdef", admin)

    def test_prepare_writes_bounded_secret_free_funding_request(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            recovery_path = root / "recovery.json"
            funding_path = root / "funding.json"
            MODULE.prepare(
                SimpleNamespace(recovery_file=recovery_path, funding_request=funding_path)
            )
            recovery = json.loads(recovery_path.read_text(encoding="utf-8"))
            funding = json.loads(funding_path.read_text(encoding="utf-8"))

            self.assertEqual(recovery["schema_version"], MODULE.RECOVERY_SCHEMA)
            self.assertEqual(set(recovery["actors"]), set(MODULE.ACTORS))
            self.assertEqual(funding["schema_version"], MODULE.FUNDING_SCHEMA)
            self.assertEqual(funding["network"], MODULE.NETWORK)
            self.assertEqual(funding["chain_id"], MODULE.CHAIN_ID)
            self.assertEqual(funding["from"], MODULE.ADMIN)
            self.assertEqual(funding["recipient"], recovery["actors"]["keeper"]["address"])
            self.assertEqual(funding["eth_wei"], MODULE.Runner.ADMIN_FUNDING_ETH_WEI)
            self.assertEqual(funding["usdc_base_units"], MODULE.Runner.ADMIN_FUNDING_USDC)
            self.assertEqual(funding["maximum_transactions"], 2)
            self.assertNotIn("private_key", json.dumps(funding))

    def test_recovery_round_trip_preserves_distinct_actor_addresses(self):
        with tempfile.TemporaryDirectory() as directory:
            path, expected = MODULE.create_recovery(Path(directory) / "recovery.json")
            observed, actors = MODULE.load_recovery(path)

            self.assertEqual(observed["user_salt"], expected["user_salt"])
            addresses = {actor.address.lower() for actor in actors.values()}
            self.assertEqual(len(addresses), len(MODULE.ACTORS))
            self.assertTrue(all(len(row["private_key"].removeprefix("0x")) == 64 for row in observed["actors"].values()))

    def test_funding_request_is_small_relative_to_admin_rehearsal_balance(self):
        self.assertLessEqual(MODULE.Runner.ADMIN_FUNDING_USDC, 500_000)
        self.assertLessEqual(MODULE.Runner.ADMIN_FUNDING_ETH_WEI, 500_000_000_000_000)
        required_usdc = (
            MODULE.Runner.TARGET * 2
            + MODULE.Runner.VERIFIER_REWARD * 2
            + MODULE.Runner.VERIFIER_REWARD
            + MODULE.Runner.VERIFIER_REWARD
        )
        self.assertLessEqual(required_usdc, MODULE.Runner.ADMIN_FUNDING_USDC)


if __name__ == "__main__":
    unittest.main()
