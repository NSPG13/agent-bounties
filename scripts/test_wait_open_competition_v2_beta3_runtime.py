import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).with_name("wait_open_competition_v2_beta3_runtime.py")
SPEC = importlib.util.spec_from_file_location("wait_open_competition_v2_beta3_runtime", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


class RuntimeWaitTests(unittest.TestCase):
    def fixture(self):
        expected = {key: f"value-{key}" for key in MODULE.IDENTITY_KEYS}
        expected["factory_contract"] = "0x" + "11" * 20
        response = {
            "release": {**expected, "proof_broker_enabled": True, "public_creation_enabled": False},
            "indexer_agreement": {"agrees": True, "factory_contract": expected["factory_contract"]},
        }
        return expected, response

    def test_exact_runtime_requires_identity_flags_and_agreement(self):
        expected, response = self.fixture()
        self.assertTrue(MODULE.exact_runtime(response, expected, True, False))
        response["release"]["release_hash"] = "drift"
        self.assertFalse(MODULE.exact_runtime(response, expected, True, False))

    def test_exact_runtime_rejects_missing_agreement(self):
        expected, response = self.fixture()
        response["indexer_agreement"] = None
        self.assertFalse(MODULE.exact_runtime(response, expected, True, False))


if __name__ == "__main__":
    unittest.main()
