import copy
import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("audit_open_competition_v1_rehearsal.py")
SPEC = importlib.util.spec_from_file_location("open_competition_audit", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def hash_of(byte: str) -> str:
    return "0x" + byte * 64


def address_of(byte: str) -> str:
    return "0x" + byte * 40


def event(name: str, byte: str, index: int) -> dict:
    return {
        "name": name,
        "transaction_hash": hash_of(byte),
        "block_number": 100 + index,
        "log_index": index,
    }


def fixtures() -> tuple[dict, dict]:
    compiler = {"solc": "0.8.26", "optimizer": True, "optimizer_runs": 200, "evm": "cancun"}
    bundle = {
        "schema_version": "agent-bounties/open-competition-v1-deployment-bundle-v1",
        "protocol_version": MODULE.PROTOCOL,
        "network": "base-sepolia",
        "chain_id": 84532,
        "source_commit": "a" * 40,
        "compiler": compiler,
        "deployer": MODULE.ADMIN,
        "settlement_token": MODULE.USDC,
        "actions": [
            {
                "name": "deploy_leading_zero_work_verifier_16",
                "expected_contract": address_of("1"),
                "runtime_code_hash": hash_of("1"),
            },
            {
                "name": "deploy_open_competition_factory_v1",
                "expected_contract": address_of("2"),
                "runtime_code_hash": hash_of("2"),
                "expected_implementation": address_of("3"),
                "implementation_runtime_code_hash": hash_of("3"),
            },
        ],
    }
    deployment = lambda address, runtime, tx: {
        "address": address,
        "transaction_hash": hash_of(tx),
        "block_number": 90,
        "block_hash": hash_of("f"),
        "runtime_code_hash": runtime,
        "runtime_matches": True,
    }
    rehearsal = {
        "schema_version": MODULE.SCHEMA,
        "protocol_version": MODULE.PROTOCOL,
        "network": "base-sepolia",
        "chain_id": 84532,
        "deployment_state": "sepolia_rehearsed_not_ready_to_earn",
        "public_inventory_eligible": False,
        "source_commit": "a" * 40,
        "compiler": compiler,
        "deployer": MODULE.ADMIN,
        "settlement_token": MODULE.USDC,
        "deployments": {
            "verifier": deployment(address_of("1"), hash_of("1"), "a"),
            "factory": deployment(address_of("2"), hash_of("2"), "b"),
            "implementation": deployment(address_of("3"), hash_of("3"), "b"),
        },
        "actors": {
            "creator": address_of("4"),
            "failed_competitor": address_of("5"),
            "passing_competitor": address_of("6"),
            "expiring_competitor": address_of("7"),
            "relayer": address_of("8"),
        },
        "scenarios": {
            "settlement_and_losing_bond_withdrawal": {
                "bounty_contract": address_of("9"),
                "transactions": [hash_of("4")],
                "events": [event("BountySettled", "4", 0), event("EntryBondWithdrawn", "5", 1)],
                "balance_deltas": {"escrow": 0},
                "reconciled": True,
                "assertions": {"failed_entry_preserved_reward": True, "losing_bond_returned": True},
            },
            "expiry_cancellation_and_refund": {
                "bounty_contract": address_of("a"),
                "transactions": [hash_of("6")],
                "events": [
                    event("CommitmentExpired", "6", 0),
                    event("BountyCancelled", "7", 1),
                    event("RefundWithdrawn", "8", 2),
                ],
                "balance_deltas": {"escrow": 0},
                "reconciled": True,
                "assertions": {"principal_refunded": True, "expired_bond_refunded_as_bonus": True},
            },
        },
        "adversarial_checks": {name: True for name in MODULE.REQUIRED_ADVERSARIAL},
        "bytecode_freeze": {
            "verifier_runtime_code_hash": hash_of("1"),
            "factory_runtime_code_hash": hash_of("2"),
            "implementation_runtime_code_hash": hash_of("3"),
        },
        "evidence_boundary": "This fixture proves only that the fail-closed rehearsal manifest audit is internally consistent.",
    }
    return bundle, rehearsal


class RehearsalAuditTests(unittest.TestCase):
    def test_complete_manifest_passes_but_remains_out_of_public_inventory(self) -> None:
        bundle, rehearsal = fixtures()
        report = MODULE.audit(bundle, rehearsal)
        self.assertTrue(report["passed"])
        self.assertFalse(report["public_inventory_eligible"])

    def test_runtime_drift_fails_closed(self) -> None:
        bundle, rehearsal = fixtures()
        rehearsal["deployments"]["factory"]["runtime_code_hash"] = hash_of("d")
        with self.assertRaisesRegex(MODULE.RehearsalAuditError, "factory runtime hash mismatch"):
            MODULE.audit(bundle, rehearsal)

    def test_public_inventory_activation_fails_closed(self) -> None:
        bundle, rehearsal = fixtures()
        rehearsal["public_inventory_eligible"] = True
        with self.assertRaisesRegex(MODULE.RehearsalAuditError, "outside public inventory"):
            MODULE.audit(bundle, rehearsal)

    def test_secret_bearing_fields_are_rejected(self) -> None:
        bundle, rehearsal = fixtures()
        rehearsal["actors"]["private_key"] = "do-not-store"
        with self.assertRaisesRegex(MODULE.RehearsalAuditError, "secret-bearing field"):
            MODULE.audit(bundle, rehearsal)

    def test_cancelled_scenario_cannot_claim_payment(self) -> None:
        bundle, rehearsal = fixtures()
        bad = copy.deepcopy(rehearsal)
        bad["scenarios"]["expiry_cancellation_and_refund"]["events"].append(
            event("BountySettled", "9", 3)
        )
        with self.assertRaisesRegex(MODULE.RehearsalAuditError, "cannot contain settlement"):
            MODULE.audit(bundle, bad)


if __name__ == "__main__":
    unittest.main()
