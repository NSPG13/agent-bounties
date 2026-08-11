import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("open_competition_v2_proof_rehearsal.py")
SPEC = importlib.util.spec_from_file_location("open_competition_v2_proof_rehearsal", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OpenCompetitionV2ProofRehearsalTests(unittest.TestCase):
    def test_parameter_schema_has_only_static_fields(self) -> None:
        self.assertEqual(len(MODULE.PARAM_TYPES), 17)
        self.assertNotIn("bytes", MODULE.PARAM_TYPES)
        self.assertEqual(MODULE.PARAM_TYPES[6], "int256")

    def test_function_calldata_matches_erc20_selector(self) -> None:
        data = MODULE.function_data(
            "approve(address,uint256)",
            ["address", "uint256"],
            [MODULE.CREATOR, 7],
        )
        self.assertTrue(data.startswith("0x095ea7b3"))
        self.assertEqual(len(bytes.fromhex(data[2:])), 68)

    def test_canonical_settlement_topic_is_bytes32(self) -> None:
        self.assertRegex(MODULE.SETTLED_TOPIC, r"^0x[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
