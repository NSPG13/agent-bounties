import unittest
from unittest.mock import patch

import rehearse_open_competition_v2_beta3_prover_service as rehearsal


class ProductionProverRehearsalTests(unittest.TestCase):
    def test_post_binds_bearer_and_idempotency_headers(self) -> None:
        response = unittest.mock.MagicMock()
        response.__enter__.return_value.read.return_value = b'{"status":"pending"}'
        with patch.object(rehearsal, "urlopen", return_value=response) as opened:
            value = rehearsal.post("http://127.0.0.1:9070/v1/prove", "secret", "repair:1", {"x": 1})
        request = opened.call_args.args[0]
        self.assertEqual(value, {"status": "pending"})
        self.assertEqual(request.headers["Authorization"], "Bearer secret")
        self.assertEqual(request.headers["Idempotency-key"], "repair:1")


if __name__ == "__main__":
    unittest.main()
