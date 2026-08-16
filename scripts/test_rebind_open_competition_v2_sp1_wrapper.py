import hashlib
import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("rebind_open_competition_v2_sp1_wrapper.py")
SPEC = importlib.util.spec_from_file_location("v2_sp1_wrapper", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class Sp1WrapperTests(unittest.TestCase):
    def source(self) -> str:
        return '''import {Groth16Verifier} from "./Groth16Verifier.sol";
contract SP1Verifier is Groth16Verifier {
function VERSION() external pure returns (string memory) {
return "agent-bounties-sp1-safe-v5";
}
function VERIFIER_HASH() public pure returns (bytes32) {
return 0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa;
}
}'''

    def test_rebinds_only_the_verifier_hash(self) -> None:
        result, value = MODULE.rebind(self.source(), b"trusted-vk", "groth16")
        expected = "0x" + hashlib.sha256(b"trusted-vk").hexdigest()
        self.assertEqual(value, expected)
        self.assertIn(f"return {expected};", result)
        self.assertIn("agent-bounties-sp1-safe-v5", result)

    def test_rejects_wrong_circuit_or_wrapper(self) -> None:
        with self.assertRaisesRegex(ValueError, "missing exact fragment"):
            MODULE.rebind(self.source().replace("safe-v5", "safe-v3"), b"vk", "groth16")
        with self.assertRaisesRegex(ValueError, "exactly one verifier hash"):
            MODULE.rebind(self.source().replace("0x" + "aa" * 32, "pending"), b"vk", "groth16")


if __name__ == "__main__":
    unittest.main()
