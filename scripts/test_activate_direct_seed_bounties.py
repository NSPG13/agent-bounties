#!/usr/bin/env python3

from __future__ import annotations

from pathlib import Path
import sys
import unittest


SCRIPTS = Path(__file__).resolve().parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))

import activate_direct_seed_bounties as MODULE


class ActivateDirectSeedBountiesTests(unittest.TestCase):
    def test_exact_profitable_economics_and_scope(self) -> None:
        self.assertEqual(MODULE.SOLVER_REWARD, 1_990_000)
        self.assertEqual(MODULE.VERIFIER_REWARD, 10_000)
        self.assertEqual(MODULE.TARGET, 2_000_000)
        self.assertEqual(sorted(MODULE.ISSUES), [634, 635, 636, 637, 638])
        self.assertEqual(len(MODULE.ISSUES) * MODULE.TARGET, 10_000_000)

    def test_terms_are_static_signed_quorum_documents(self) -> None:
        documents = {
            issue: MODULE.terms_document(issue, config)
            for issue, config in MODULE.ISSUES.items()
        }
        self.assertEqual(len({item["contract_terms"]["creation_nonce"] for item in documents.values()}), 5)
        for issue, item in documents.items():
            terms = item["contract_terms"]
            policy = item["verification_policy"]
            self.assertEqual(terms["creator_wallet"], MODULE.WALLET)
            self.assertEqual(terms["solver_reward"]["amount"], 1_990_000)
            self.assertEqual(terms["verifier_reward"]["amount"], 10_000)
            self.assertEqual(terms["claim_bond"]["amount"], 10_000)
            self.assertEqual(terms["initial_funding"]["amount"], 2_000_000)
            self.assertEqual(policy["mechanism"], "signed_quorum")
            self.assertEqual(policy["verifiers"], MODULE.SIGNED_QUORUM_VERIFIERS)
            self.assertEqual(policy["threshold"], 2)
            self.assertIn(str(issue), item["source_url"])

    def test_creation_payload_matches_committed_terms(self) -> None:
        document = MODULE.terms_document(634, MODULE.ISSUES[634])
        published = {
            "terms_hash": "0x" + "11" * 32,
            "policy_hash": "0x" + "12" * 32,
            "acceptance_criteria_hash": "0x" + "13" * 32,
            "benchmark_hash": "0x" + "14" * 32,
            "evidence_schema_hash": "0x" + "15" * 32,
        }
        payload = MODULE.creation_payload(document, published)
        self.assertEqual(payload["verification_mode"], "signed_quorum")
        self.assertIsNone(payload["verifier_module"])
        self.assertIsNone(payload["verifier_reward_recipient"])
        self.assertEqual(payload["initial_funding"]["amount"], 2_000_000)
        self.assertEqual(payload["creation_nonce"], document["contract_terms"]["creation_nonce"])

    def test_resume_checks_canonical_state_before_planning_or_send(self) -> None:
        source = (SCRIPTS / "activate_direct_seed_bounties.py").read_text(encoding="utf-8")
        canonical = source.index('cast.call(FACTORY, "isCanonicalBounty(address)(bool)", predicted)')
        planner = source.index('"scripts/plan_bounded_agent_action.py"', canonical)
        sender = source.index("cast.send_data(", planner)
        self.assertLess(canonical, planner)
        self.assertLess(planner, sender)

    def test_live_issue_copy_does_not_overclaim_net_profit(self) -> None:
        body = MODULE.issue_body(
            634,
            MODULE.ISSUES[634],
            {
                "contract": "0x" + "21" * 20,
                "transaction_hash": "0x" + "22" * 32,
            },
        )
        self.assertIn("1.99 USDC", body)
        self.assertIn("0.01 USDC", body)
        self.assertIn("Required external spend: **0 USDC**", body)
        self.assertNotIn("guaranteed net profit", body.lower())
        self.assertIn("Only canonical `BountySettled` proves payment", body)


if __name__ == "__main__":
    unittest.main()
