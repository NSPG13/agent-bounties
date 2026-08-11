import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("build_open_competition_v2_beta1_release.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_release", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2ReleaseTests(unittest.TestCase):
    def test_metric_identity_is_single_release_source_of_truth(self) -> None:
        identity = MODULE.METRIC_IDENTITY
        self.assertEqual(
            identity["schema"],
            "agent-bounties/open-competition-v2-metric-release-identity-v1",
        )
        self.assertEqual(identity["rust_version"], "1.96.1")
        self.assertEqual(MODULE.PROGRAM_VKEY, identity["program_vkey"])
        self.assertEqual(MODULE.SOURCE_HASH, identity["source_hash"])
        self.assertEqual(MODULE.ELF_HASH, identity["elf_keccak256"])
        self.assertEqual(MODULE.ELF_SHA256, identity["elf_sha256"])

    def test_current_manifest_fails_closed(self) -> None:
        gates = MODULE.load_gates(
            MODULE.ROOT / "deployments/open-competition-v2-beta1-release-gates.json"
        )
        self.assertFalse(gates["prelaunch_complete"])
        self.assertFalse(gates["public_beta_launch_complete"])
        self.assertFalse(gates["graduation_complete"])
        self.assertRegex(gates["beta_risk_hash"], r"^0x[0-9a-f]{64}$")

    def test_release_stages_do_not_require_graduation_before_beta(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-staged-gates.json"
        evidence = {
            "source_commit": "a" * 40,
            "evidence_hash": "0x" + "11" * 32,
            "uri": "https://example.test/evidence",
        }
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta1-release-gates-v2",
            "protocol_version": "agent-bounties/open-competition-v2-beta1",
            "beta_risk_preimage": "risk",
            "gates": {name: False for name in MODULE.REQUIRED_GATE_NAMES},
            "evidence": {name: None for name in MODULE.REQUIRED_GATE_NAMES},
        }
        path.parent.mkdir(parents=True, exist_ok=True)
        for name in MODULE.PRELAUNCH_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path)
        self.assertTrue(gates["prelaunch_complete"])
        self.assertFalse(gates["public_beta_launch_complete"])
        self.assertFalse(gates["graduation_complete"])
        preflight = {
            "number": 1,
            "hash": "0x" + "22" * 32,
            "timestamp": 1,
            "deployer_nonce": 1,
            "deployer_eth_wei": MODULE.MIN_DEPLOYER_ETH_WEI,
            "deployer_usdc_base_units": 0,
            "dependency_runtime_hashes": {},
        }
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            preflight=preflight,
            gates=gates,
        )
        self.assertTrue(bundle["activation"]["mainnet_signing_allowed"])
        self.assertFalse(bundle["activation"]["public_creation_enabled"])
        self.assertFalse(bundle["activation"]["default_protocol_enabled"])

        for name in MODULE.PUBLIC_BETA_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path)
        self.assertTrue(gates["public_beta_launch_complete"])
        self.assertFalse(gates["graduation_complete"])
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            preflight=preflight,
            gates=gates,
        )
        self.assertTrue(bundle["activation"]["public_creation_enabled"])
        self.assertFalse(bundle["activation"]["default_protocol_enabled"])

        for name in MODULE.GRADUATION_GATE_NAMES:
            value["gates"][name] = True
            value["evidence"][name] = evidence
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        gates = MODULE.load_gates(path)
        self.assertTrue(gates["graduation_complete"])
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            preflight=preflight,
            gates=gates,
        )
        self.assertTrue(bundle["activation"]["default_protocol_enabled"])

    def test_completed_gate_requires_hash_bound_evidence(self) -> None:
        path = MODULE.ROOT / "target/tmp/open-competition-v2-unevidenced-gate.json"
        gates = {name: False for name in MODULE.REQUIRED_GATE_NAMES}
        gates["repository_gate_complete"] = True
        value = {
            "schema_version": "agent-bounties/open-competition-v2-beta1-release-gates-v2",
            "protocol_version": "agent-bounties/open-competition-v2-beta1",
            "beta_risk_preimage": "risk",
            "gates": gates,
            "evidence": {name: None for name in MODULE.REQUIRED_GATE_NAMES},
        }
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(__import__("json").dumps(value), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "lacks evidence"):
            MODULE.load_gates(path)

    def test_official_route_decoder_rejects_malformed_data(self) -> None:
        with self.assertRaisesRegex(ValueError, "two ABI words"):
            MODULE.decode_route("0x00")
        encoded = "0x" + (
            MODULE.address_word(MODULE.GROTH16_VERIFIER) + bytes(32)
        ).hex()
        self.assertEqual(
            MODULE.decode_route(encoded), (MODULE.GROTH16_VERIFIER, False)
        )

    def test_pinned_addresses_match_factory_source(self) -> None:
        source = (
            MODULE.ROOT
            / "contracts/base-escrow/src/OpenCompetitionBountyFactoryV2Beta1.sol"
        ).read_text(encoding="utf-8").lower()
        for address in (
            MODULE.GROTH16_GATEWAY,
            MODULE.PLONK_GATEWAY,
            MODULE.GROTH16_VERIFIER,
            MODULE.PLONK_VERIFIER,
            MODULE.NETWORKS["base-mainnet"]["usdc"],
            MODULE.NETWORKS["base-sepolia"]["usdc"],
        ):
            self.assertIn(address.removeprefix("0x"), source)

    def test_immutable_names_are_bound_by_ast_not_sort_order(self) -> None:
        artifact = MODULE.artifact(
            "OpenCompetitionBountyFactoryV2Beta1", "OpenCompetitionBountyFactoryV2Beta1"
        )
        self.assertEqual(
            set(MODULE.immutable_names(artifact).values()),
            {"settlementToken", "groth16Adapter", "plonkAdapter", "implementation"},
        )

    def test_deployer_does_not_need_to_hold_canary_usdc(self) -> None:
        gates = MODULE.load_gates(
            MODULE.ROOT / "deployments/open-competition-v2-beta1-release-gates.json"
        )
        preflight = {
            "number": 1,
            "hash": "0x" + "11" * 32,
            "timestamp": 1,
            "deployer_nonce": 1,
            "deployer_eth_wei": MODULE.MIN_DEPLOYER_ETH_WEI,
            "deployer_usdc_base_units": MODULE.CANARY_BUDGET - 1,
            "dependency_runtime_hashes": {},
        }
        bundle = MODULE.build_bundle(
            network_name="base-mainnet",
            deployer=MODULE.DEFAULT_DEPLOYER,
            source_commit="a" * 40,
            preflight=preflight,
            gates=gates,
        )
        self.assertTrue(bundle["canary_budget"]["deployer_is_not_required_to_fund"])
        self.assertEqual(
            bundle["canary_budget"]["required_usdc_base_units"], MODULE.CANARY_BUDGET
        )
        self.assertEqual(bundle["compiler"]["solc"], "0.8.26+commit.8a97fa7a")
        self.assertEqual(bundle["compiler"]["image"], MODULE.SOLC_IMAGE)
        self.assertRegex(MODULE.SOLC_IMAGE, r"^docker\.io/ethereum/solc@sha256:[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
