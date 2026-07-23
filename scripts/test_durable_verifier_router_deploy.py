#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("durable_verifier_router_deploy.py")
SPEC = importlib.util.spec_from_file_location("durable_verifier_router_deploy", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class DurableVerifierRouterDeployTests(unittest.TestCase):
    def test_materialized_terms_share_router_policy_and_unique_nonces(self) -> None:
        router = "0x" + "12" * 20
        documents = MODULE.materialize_terms(Path(__file__).resolve().parents[1], router)
        self.assertEqual(sorted(documents), [333, 334, 335, 336])
        nonces = set()
        policies = []
        for issue, document in documents.items():
            self.assertEqual(document["verification_policy"]["verifier_module"], router)
            self.assertEqual(document["contract_terms"]["initial_funding"]["amount"], 2_010_000)
            self.assertEqual(document["contract_terms"]["claim_bond"]["amount"], 10_000)
            self.assertIn(str(issue), document["source_url"])
            nonces.add(document["contract_terms"]["creation_nonce"])
            policies.append(document["verification_policy"])
        self.assertEqual(len(nonces), 4)
        self.assertTrue(all(policy == policies[0] for policy in policies))

    def test_create_payload_uses_published_commitments_and_router(self) -> None:
        router = "0x" + "34" * 20
        document = MODULE.materialize_terms(Path(__file__).resolve().parents[1], router)[333]
        published = {
            "terms_hash": "0x" + "01" * 32,
            "policy_hash": "0x" + "02" * 32,
            "acceptance_criteria_hash": "0x" + "03" * 32,
            "benchmark_hash": "0x" + "04" * 32,
            "evidence_schema_hash": "0x" + "05" * 32,
        }
        payload = MODULE.create_payload(document, published)
        self.assertEqual(payload["verifier_module"], router)
        self.assertEqual(payload["policy_hash"], published["policy_hash"])
        self.assertEqual(payload["threshold"], 1)
        self.assertEqual(payload["verifiers"], [])
        self.assertEqual(payload["initial_funding"]["amount"], 2_010_000)

    def test_redaction_hides_private_key_and_rpc_value(self) -> None:
        rendered = MODULE.redact_command(
            ["cast", "send", "--private-key", "secret", "--rpc-url", "credentialed", "--json"]
        )
        self.assertNotIn("secret", rendered)
        self.assertNotIn("credentialed", rendered)
        self.assertEqual(rendered.count("***"), 2)

    def test_markdown_reports_durable_signature_boundary(self) -> None:
        report = {
            "predicted_router": "0x" + "56" * 20,
            "router_already_deployed": False,
            "registrar": MODULE.KEEPER,
            "guardian": MODULE.OWNER,
            "activation_delay_seconds": MODULE.ROUTER_ACTIVATION_DELAY,
            "bounded_wallet": {
                "address": MODULE.BOUNDED_WALLET,
                "usdc_balance_base_units": 37_000_000,
                "policy": {"deterministic_verifier": "0x" + "78" * 20},
            },
            "replacement_economics": {"total_funding_required": 8_040_000},
        }
        value = MODULE.markdown(report)
        self.assertIn("7 days", value)
        self.assertIn("28.960000 USDC", value)
        self.assertIn("cannot move funds", value)
        self.assertIn("not deployment", value)


if __name__ == "__main__":
    unittest.main()
