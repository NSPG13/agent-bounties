import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch


PATH = Path(__file__).with_name("run_open_competition_v2_sepolia_rehearsal.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_sepolia_rehearsal", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class SepoliaRehearsalTests(unittest.TestCase):
    def test_signed_rpc_checksummed_lowercase_contract_destination(self):
        destination = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"

        class Actor:
            address = "0xfd7bE4C69541aB297aEcE2a674fc1418b898cC0a"

            def sign_transaction(self, transaction):
                self.transaction = transaction
                return SimpleNamespace(raw_transaction=b"signed")

        def rpc_response(_url, method, _params):
            return {
                "eth_chainId": hex(MODULE.CHAIN_ID),
                "eth_getTransactionCount": "0x0",
                "eth_getBlockByNumber": {"baseFeePerGas": "0x1"},
                "eth_maxPriorityFeePerGas": "0x1",
                "eth_estimateGas": "0x5208",
                "eth_sendRawTransaction": "0x" + "44" * 32,
            }[method]

        actor = Actor()
        with patch.object(MODULE, "rpc", side_effect=rpc_response):
            client = MODULE.SignedRpc("https://base-sepolia.invalid")
            with patch.object(client, "wait_receipt", return_value={"status": "0x1"}):
                client.send(actor, to=destination)

        self.assertEqual(
            actor.transaction["to"],
            "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
        )

    def test_prepared_actor_roles_normalize_creator_to_deployer(self):
        actors = {
            "deployer": "0x" + "11" * 20,
            "solver_a": "0x" + "22" * 20,
            "solver_b": "0x" + "33" * 20,
        }
        self.assertEqual(
            MODULE.prepared_actor_set(actors),
            {
                "creator": actors["deployer"],
                "solver_a": actors["solver_a"],
                "solver_b": actors["solver_b"],
            },
        )

    def test_sepolia_rebind_deploys_exact_three_component_sequence(self):
        deployer = "0x" + "11" * 20
        predicted = ["0x" + f"{value:040x}" for value in (101, 102, 103)]
        transactions = [
            {
                "component": component,
                "from_nonce": 9 + offset,
                "predicted_address": predicted[offset],
                "data": "0x" + f"{offset + 1:02x}",
            }
            for offset, component in enumerate(
                ("groth16_verifier", "plonk_verifier", "factory")
            )
        ]
        raw_bundle = {
            "network": "base-sepolia",
            "chain_id": 84532,
            "deployer": deployer,
            "preflight_safe_block": {"deployer_nonce": 4},
            "factory": {
                "address": "0x" + "22" * 20,
                "runtime_code_hash": "0x" + "33" * 32,
            },
        }

        def rebuild(_bundle, nonce, _assets):
            return {
                **raw_bundle,
                "preflight_safe_block": {"deployer_nonce": nonce},
                "factory": {
                    "address": "0x" + f"{nonce + 500:040x}",
                    "runtime_code_hash": "0x" + "44" * 32,
                },
                "deployment_transactions": transactions,
            }

        class Client:
            url = "https://base-sepolia.invalid"

            def __init__(self):
                self.sent = []

            def send(self, _signer, *, data):
                offset = len(self.sent)
                self.sent.append(data)
                return {
                    "contractAddress": predicted[offset],
                    "transactionHash": "0x" + f"{offset + 1:064x}",
                }

        signer = type("Signer", (), {"address": deployer})()
        client = Client()
        with patch.object(MODULE, "rpc", return_value=hex(9)), patch.object(
            MODULE, "runtime_hash", return_value=("0x" + "00" * 32, 0)
        ), patch.object(MODULE, "bundle_for_nonce", side_effect=rebuild):
            resolved, receipts = MODULE.resolve_or_deploy_factory(
                client, signer, raw_bundle, {"pinned": True}
            )

        self.assertEqual(resolved["preflight_safe_block"]["deployer_nonce"], 9)
        self.assertEqual(client.sent, [item["data"] for item in transactions])
        self.assertEqual(list(receipts), [item["component"] for item in transactions])

    def test_component_verification_includes_external_verifiers(self):
        keys = (
            "groth16_verifier",
            "plonk_verifier",
            "factory",
            "groth16_adapter",
            "plonk_adapter",
            "implementation",
        )
        bundle = {
            key: {
                "address": "0x" + f"{index + 1:040x}",
                "runtime_code_hash": "0x" + f"{index + 1:064x}",
                "runtime_code_bytes": index + 10,
            }
            for index, key in enumerate(keys)
        }

        def observed(_url, address):
            item = next(value for value in bundle.values() if value["address"] == address)
            return item["runtime_code_hash"], item["runtime_code_bytes"]

        with patch.object(MODULE, "runtime_hash", side_effect=observed):
            self.assertEqual(set(MODULE.verify_components("unused", bundle)), set(keys))

    def test_every_release_rehearsal_call_pins_generated_verifier_assets(self):
        workflow = (
            Path(__file__).resolve().parents[1]
            / ".github"
            / "workflows"
            / "open-competition-v2-beta3-release.yml"
        ).read_text(encoding="utf-8")
        lines = workflow.splitlines()
        calls = [
            index
            for index, line in enumerate(lines)
            if "python scripts/run_open_competition_v2_sepolia_rehearsal.py" in line
        ]
        self.assertEqual(len(calls), 4)
        for index in calls:
            self.assertIn(
                "--verifier-assets target/release-assets/verifier-assets.json",
                "\n".join(lines[index : index + 6]),
            )

    def test_nonce_rebind_uses_explicit_pinned_verifier_assets(self):
        bundle = {
            "preflight_safe_block": {"deployer_nonce": 7},
            "deployer": "0x" + "11" * 20,
            "source_commit": "22" * 20,
            "repository_subject": {"hash": "0x" + "33" * 32},
            "release_gates": {"pinned": True},
        }
        verifier_assets = {"proof_systems": {"pinned": True}}
        rebuilt = {"factory": {"from_nonce": 11}}

        with patch.object(
            MODULE.release,
            "load_verifier_assets",
            side_effect=AssertionError("filesystem fallback"),
        ), patch.object(MODULE.release, "build_bundle", return_value=rebuilt) as build:
            self.assertIs(MODULE.bundle_for_nonce(bundle, 9, verifier_assets), rebuilt)

        self.assertEqual(build.call_args.kwargs["preflight"]["deployer_nonce"], 9)
        self.assertIs(build.call_args.kwargs["verifier_assets"], verifier_assets)

    def test_actor_derivation_is_stable_and_scoped(self):
        key = bytes.fromhex("11" * 32)
        commit = "22" * 20
        first = MODULE.derived_actor(key, commit, "solver-a")
        second = MODULE.derived_actor(key, commit, "solver-a")
        other = MODULE.derived_actor(key, commit, "solver-b")
        self.assertEqual(first.address, second.address)
        self.assertNotEqual(first.address, other.address)

    def test_actor_derivation_is_unique_per_release_attempt(self):
        key = bytes.fromhex("11" * 32)
        commit = "22" * 20
        first = MODULE.derived_actor(key, commit, "solver-a", "run-1:attempt-1")
        retry = MODULE.derived_actor(key, commit, "solver-a", "run-1:attempt-2")
        self.assertNotEqual(first.address, retry.address)
        self.assertNotEqual(
            MODULE.actor_derivation_id(commit, "run-1:attempt-1"),
            MODULE.actor_derivation_id(commit, "run-1:attempt-2"),
        )

    def test_actor_derivation_rejects_empty_salt(self):
        with self.assertRaises(MODULE.SepoliaRehearsalError):
            MODULE.actor_derivation_id("22" * 20, "")

    def test_proof_summary_drops_sensitive_bulk_bytes(self):
        value = {
            "mode": "groth16",
            "proof_hex": "0x010203",
            "journal_hex": "0x0405",
            "elapsed_seconds": 3.5,
        }
        summary = MODULE.proof_summary(value)
        self.assertEqual(summary["proof_bytes"], 3)
        self.assertEqual(summary["journal_bytes"], 2)
        self.assertNotIn("proof_hex", summary)
        self.assertNotIn("journal_hex", summary)

    def test_private_key_validation_fails_closed(self):
        for value in ("", "0x1", "0x" + "00" * 32):
            with self.assertRaises(MODULE.SepoliaRehearsalError):
                MODULE.normalized_key(value)

    def test_x402_canary_spec_binds_artifact_to_the_journal(self):
        fixture = {
            "scope": {
                "chain_id": 84532,
                "competition": [17] * 20,
                "bounty_id": [34] * 32,
                "solver": [51] * 20,
                "solver_nonce": 3,
                "proof_system": [68] * 32,
                "program_vkey": [85] * 32,
                "source_hash": [102] * 32,
                "elf_hash": [119] * 32,
                "execution_policy_hash": [136] * 32,
                "settlement_policy_hash": [153] * 32,
                "beta_risk_hash": [170] * 32,
            },
            "mode": "maximize_exact_matches",
            "threshold": 1,
            "vectors": [{"expected": 2, "observed": 2, "weight": 1}],
        }
        spec = MODULE.x402_canary_spec(
            fixture,
            "0x" + "11" * 20,
            "0x" + "22" * 32,
            "0x" + "33" * 20,
            3,
        )
        journal = MODULE.rehearsal.expected_journal(fixture)
        self.assertEqual(spec["artifact_hash"], "0x" + journal[192:224].hex())
        self.assertEqual(spec["metric"]["threshold"], "1")


if __name__ == "__main__":
    unittest.main()
