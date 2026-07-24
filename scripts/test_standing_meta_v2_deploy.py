import json
from pathlib import Path
import tempfile
import unittest

from scripts.standing_meta_v2_deploy import (
    BASE_SEPOLIA_USDC,
    DeploymentError,
    confirmed_terms_publication,
    normalize_address,
    parse_cast_uint,
    read_broadcast,
    require_bytes32,
    wait_for_block_timestamp,
    wait_for_runtime_code,
    wait_for_later_timestamp,
)


class CodeSequence:
    def __init__(self, values: list[str]) -> None:
        self.values = values

    def code(self, _address: str) -> str:
        if len(self.values) > 1:
            return self.values.pop(0)
        return self.values[0]


class CastSequence:
    def __init__(self, values: list[object]) -> None:
        self.values = values

    def cast_run(self, *_args: str) -> str:
        if len(self.values) > 1:
            value = self.values.pop(0)
        else:
            value = self.values[0]
        if isinstance(value, Exception):
            raise value
        return str(value)


class StandingMetaV2DeployTests(unittest.TestCase):
    def test_block_timestamp_waits_for_rpc_indexing(self) -> None:
        foundry = CastSequence([DeploymentError("block not found"), "112 [1.12e2]"])
        self.assertEqual(
            wait_for_block_timestamp(
                foundry, 42, timeout_seconds=1, poll_interval_seconds=0  # type: ignore[arg-type]
            ),
            112,
        )

    def test_block_timestamp_timeout_fails_closed(self) -> None:
        with self.assertRaises(DeploymentError):
            wait_for_block_timestamp(
                CastSequence([DeploymentError("block not found")]),  # type: ignore[arg-type]
                42,
                timeout_seconds=0,
                poll_interval_seconds=0,
            )

    def test_terms_wait_uses_confirmed_publication_block(self) -> None:
        registry = "0x" + "33" * 20
        transactions = [
            {
                "to": "0x" + "22" * 20,
                "function": "transfer(address,uint256)",
                "block_number": 40,
            },
            {
                "to": registry,
                "function": "publish(bytes,(bytes32,uint64))",
                "block_number": 42,
            },
        ]
        self.assertEqual(
            confirmed_terms_publication(
                CastSequence(["112 [1.12e2]"]),  # type: ignore[arg-type]
                registry,
                transactions,
                110,
            ),
            {"block_number": 42, "timestamp": 112},
        )

    def test_terms_publication_rejects_ambiguous_evidence(self) -> None:
        with self.assertRaises(DeploymentError):
            confirmed_terms_publication(
                CastSequence(["112"]),  # type: ignore[arg-type]
                "0x" + "33" * 20,
                [],
                110,
            )

    def test_completion_waits_for_four_base_block_margin(self) -> None:
        foundry = CastSequence(["107 [1.07e2]", "108 [1.08e2]"])
        self.assertEqual(
            wait_for_later_timestamp(
                foundry,  # type: ignore[arg-type]
                100,
                timeout_seconds=1,
                poll_interval_seconds=0,
            ),
            108,
        )

    def test_runtime_code_waits_for_rpc_propagation(self) -> None:
        foundry = CodeSequence(["0x", "0x6000"])
        self.assertEqual(
            wait_for_runtime_code(
                foundry,  # type: ignore[arg-type]
                "0x" + "11" * 20,
                "test contract",
                timeout_seconds=1,
                poll_interval_seconds=0,
            ),
            "0x6000",
        )

    def test_runtime_code_timeout_fails_closed(self) -> None:
        with self.assertRaises(DeploymentError):
            wait_for_runtime_code(
                CodeSequence(["0x"]),  # type: ignore[arg-type]
                "0x" + "11" * 20,
                "test contract",
                timeout_seconds=0,
                poll_interval_seconds=0,
            )

    def test_cast_uint_accepts_foundry_annotations(self) -> None:
        self.assertEqual(parse_cast_uint("3000000 [3e6]"), 3_000_000)
        self.assertEqual(parse_cast_uint("0x2a [42]"), 42)
        self.assertEqual(parse_cast_uint("4"), 4)
        with self.assertRaises(DeploymentError):
            parse_cast_uint("[3e6]")

    def test_rehearsal_uses_canonical_base_sepolia_usdc(self) -> None:
        self.assertEqual(BASE_SEPOLIA_USDC, "0x036cbd53842c5426634e7929541ec2318f3dcf7e")

    def test_exact_address_and_bytes32_shapes(self) -> None:
        self.assertEqual(
            normalize_address("0x" + "Aa" * 20, "address"),
            "0x" + "aa" * 20,
        )
        self.assertEqual(require_bytes32("0x" + "BB" * 32, "hash"), "0x" + "bb" * 32)
        with self.assertRaises(DeploymentError):
            normalize_address("0x1234", "address")
        with self.assertRaises(DeploymentError):
            require_bytes32("0x" + "00" * 31, "hash")

    def test_broadcast_requires_aligned_successful_receipts(self) -> None:
        transaction_hash = "0x" + "11" * 32
        payload = {
            "transactions": [
                {
                    "hash": transaction_hash,
                    "function": "claim()",
                }
            ],
            "receipts": [
                {
                    "status": "0x1",
                    "transactionHash": transaction_hash,
                    "from": "0x" + "22" * 20,
                    "to": "0x" + "33" * 20,
                    "blockNumber": "0x2a",
                    "logs": [],
                }
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            parsed = read_broadcast(path)
            self.assertEqual(parsed[0]["transaction_hash"], transaction_hash)
            self.assertEqual(parsed[0]["block_number"], 42)
            payload["receipts"][0]["status"] = "0x0"
            path.write_text(json.dumps(payload), encoding="utf-8")
            with self.assertRaises(DeploymentError):
                read_broadcast(path)


if __name__ == "__main__":
    unittest.main()
