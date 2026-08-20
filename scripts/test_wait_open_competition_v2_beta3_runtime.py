import importlib.util
import io
from pathlib import Path
import unittest
from unittest import mock
from urllib.error import HTTPError


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

    def test_fetch_uses_only_the_supported_network_query(self):
        response = mock.MagicMock()
        response.read.return_value = b'{"ok":true}'
        response.__enter__.return_value = response
        with mock.patch.object(MODULE, "urlopen", return_value=response) as urlopen:
            result = MODULE.fetch("https://api.example/release?network=stale")

        self.assertEqual(result, {"ok": True})
        request = urlopen.call_args.args[0]
        self.assertEqual(request.full_url, "https://api.example/release?network=base-mainnet")
        self.assertEqual(request.get_header("Cache-control"), "no-store")
        self.assertNotIn("_probe", request.full_url)

    def test_wait_reports_typed_transport_failure(self):
        error = HTTPError("https://api.example", 400, "bad request", {}, io.BytesIO())
        with mock.patch.object(MODULE, "fetch", side_effect=error), mock.patch.object(
            MODULE.time, "monotonic", side_effect=[0.0, 0.0, 1.0]
        ), mock.patch.object(MODULE.time, "sleep"):
            with self.assertRaisesRegex(MODULE.RuntimeWaitError, "http_status=400"):
                MODULE.wait("https://api.example", {}, False, False, 0.5, 0.01)


if __name__ == "__main__":
    unittest.main()
