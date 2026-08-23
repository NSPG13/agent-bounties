import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from eth_account import Account
from eth_account.messages import encode_typed_data


PATH = Path(__file__).with_name("run_open_competition_v2_x402_rehearsal.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_x402_rehearsal", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class X402RehearsalTests(unittest.TestCase):
    def test_reclaim_client_keeps_the_configured_shadow_rpc(self):
        source = PATH.read_text(encoding="utf-8")
        self.assertIn('parser.add_argument("--shadow-rpc-url")', source)
        self.assertIn(
            "sepolia.SignedRpc(args.rpc_url, args.shadow_rpc_url)", source
        )

    def test_fresh_quote_waits_for_active_canonical_projection(self):
        competition = "0x" + "11" * 20
        active = {
            "competitions": [
                {
                    "record": {
                        "projection": {
                            "competition": competition,
                            "state": "active",
                        }
                    }
                }
            ]
        }
        with patch.object(
            MODULE,
            "request_json",
            side_effect=[(200, {"competitions": []}, {}), (200, active, {})],
        ) as request:
            with patch.object(MODULE.time, "sleep") as sleep:
                MODULE.wait_for_active_competition(
                    "https://api.example.test", "base-mainnet", competition, float("inf")
                )
        self.assertEqual(request.call_count, 2)
        sleep.assert_called_once_with(3)

    def test_fresh_quote_rejects_terminal_projection(self):
        competition = "0x" + "11" * 20
        inventory = {
            "competitions": [
                {
                    "record": {
                        "projection": {
                            "competition": competition,
                            "state": "expired",
                        }
                    }
                }
            ]
        }
        with patch.object(
            MODULE, "request_json", return_value=(200, inventory, {})
        ):
            with self.assertRaisesRegex(
                MODULE.X402RehearsalError, "became expired"
            ):
                MODULE.wait_for_active_competition(
                    "https://api.example.test", "base-mainnet", competition, float("inf")
                )

    def test_resumable_job_requires_exact_paid_scope(self):
        spec = {
            "competition": "0x" + "11" * 20,
            "solver": "0x" + "22" * 20,
            "solver_nonce": "7",
            "artifact_hash": "0x" + "33" * 32,
            "proof_system": "groth16",
            "metric": {
                "mode": "maximize_exact_matches",
                "threshold": "1",
                "vectors": [{"expected": 1, "observed": 1, "weight": 1}],
            },
        }
        job = {
            "network": "base-mainnet",
            "competition_contract": spec["competition"],
            "solver": spec["solver"],
            "solver_nonce": spec["solver_nonce"],
            "artifact_hash": spec["artifact_hash"],
            "proof_system": spec["proof_system"],
            "requested_relay": True,
            "state": "paid",
            "payment_evidence": {"transaction_hash": "0x" + "44" * 32},
            "expected_public_values": "0x01",
            "program_input": spec["metric"],
        }
        MODULE.validate_resumable_job(job, spec, "base-mainnet")
        job["solver_nonce"] = "8"
        with self.assertRaisesRegex(MODULE.X402RehearsalError, "solver_nonce"):
            MODULE.validate_resumable_job(job, spec, "base-mainnet")

    def test_payment_reconciliation_recovers_ambiguous_503_without_signature(self):
        evidence = {"payment_evidence": {"transaction_hash": "0x" + "11" * 32}}
        responses = [
            (503, {}, {}),
            (202, {"state": "paid"}, {}),
            (200, evidence, {}),
        ]
        with patch.object(MODULE, "request_json", side_effect=responses) as request:
            with patch.object(MODULE.time, "sleep"):
                result = MODULE.reconcile_payment("https://example.test/payment", float("inf"))
        self.assertEqual(result, evidence)
        self.assertEqual(request.call_count, 3)
        self.assertIsNone(request.call_args_list[1].kwargs.get("headers"))
        self.assertIsNone(request.call_args_list[2].kwargs.get("headers"))

    def test_resumed_job_payment_evidence_needs_no_payment_endpoint_call(self):
        job = {"payment_evidence": {"transaction_hash": "0x" + "11" * 32}}
        payment = MODULE.resumed_payment(job)
        self.assertIs(payment["payment_evidence"], job["payment_evidence"])
        with self.assertRaises(MODULE.X402RehearsalError):
            MODULE.resumed_payment({})

    def test_payment_header_is_standard_exact_eip3009(self):
        actor = Account.from_key("0x" + "11" * 32)
        challenge = {
            "x402Version": 2,
            "resource": {"url": "https://example.test/proof"},
            "accepts": [
                {
                    "scheme": "exact",
                    "network": "eip155:84532",
                    "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
                    "amount": "100000",
                    "payTo": "0x2222222222222222222222222222222222222222",
                    "maxTimeoutSeconds": 300,
                    "extra": {
                        "assetTransferMethod": "eip3009",
                        "name": "USDC",
                        "version": "2",
                    },
                }
            ],
        }
        encoded = MODULE.sign_payment(actor, challenge)
        payload = json.loads(__import__("base64").b64decode(encoded))
        self.assertEqual(payload["accepted"]["scheme"], "exact")
        self.assertEqual(payload["payload"]["authorization"]["from"], actor.address)
        self.assertRegex(payload["payload"]["signature"], r"^0x[0-9a-f]{130}$")

    def test_payment_header_supports_base_mainnet_without_changing_asset(self):
        actor = Account.from_key("0x" + "11" * 32)
        challenge = {
            "x402Version": 2,
            "resource": {"url": "https://example.test/proof"},
            "accepts": [
                {
                    "scheme": "exact",
                    "network": "eip155:8453",
                    "asset": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                    "amount": "100000",
                    "payTo": "0x2222222222222222222222222222222222222222",
                    "maxTimeoutSeconds": 300,
                    "extra": {"assetTransferMethod": "eip3009", "name": "USDC", "version": "2"},
                }
            ],
        }
        encoded = MODULE.sign_payment(
            actor, challenge, chain_id=8453, network="eip155:8453"
        )
        payload = json.loads(__import__("base64").b64decode(encoded))
        self.assertEqual(payload["accepted"]["network"], "eip155:8453")
        self.assertEqual(payload["accepted"]["asset"], challenge["accepts"][0]["asset"])

    def test_header_decoder_rejects_non_json(self):
        with self.assertRaises(MODULE.X402RehearsalError):
            MODULE.decode_x402_header("bm90LWpzb24=")

    def test_quote_payload_uses_requested_network(self):
        spec = {
            "competition": "0x" + "11" * 20,
            "solver": "0x" + "22" * 20,
            "solver_nonce": 7,
            "artifact_hash": "0x" + "33" * 32,
            "metric": {"score": 42},
        }
        payload = MODULE.build_quote_payload(spec, "base-mainnet")
        self.assertEqual(payload["network"], "base-mainnet")
        self.assertEqual(payload["competition_contract"], spec["competition"])
        self.assertTrue(payload["relay"])

    def test_relay_authorization_signs_exact_api_typed_data(self):
        actor = Account.from_key("0x" + "11" * 32)
        authorization = {
            "types": {
                "EIP712Domain": [
                    {"name": "name", "type": "string"},
                    {"name": "version", "type": "string"},
                    {"name": "chainId", "type": "uint256"},
                    {"name": "verifyingContract", "type": "address"},
                ],
                "SubmitProof": [
                    {"name": "solver", "type": "address"},
                    {"name": "solverNonce", "type": "uint256"},
                    {"name": "publicValuesHash", "type": "bytes32"},
                    {"name": "proofHash", "type": "bytes32"},
                    {"name": "authorizationDeadline", "type": "uint256"},
                ],
            },
            "primaryType": "SubmitProof",
            "domain": {
                "name": "Agent Bounties Open Competition V2 Beta3",
                "version": "1",
                "chainId": 84532,
                "verifyingContract": "0x" + "22" * 20,
            },
            "message": {
                "solver": actor.address,
                "solverNonce": "7",
                "publicValuesHash": "0x" + "33" * 32,
                "proofHash": "0x" + "44" * 32,
                "authorizationDeadline": "2000000000",
            },
        }
        signature = MODULE.sign_relay_authorization(actor, authorization)
        recovered = Account.recover_message(
            encode_typed_data(full_message=authorization), signature=signature
        )
        self.assertEqual(recovered, actor.address)
        self.assertRegex(signature, r"^0x[0-9a-f]{130}$")

    def test_relay_authorization_rejects_stale_digest_shape(self):
        actor = Account.from_key("0x" + "11" * 32)
        with self.assertRaises(MODULE.X402RehearsalError):
            MODULE.sign_relay_authorization(actor, {"digest": "0x" + "00" * 32})


if __name__ == "__main__":
    unittest.main()
