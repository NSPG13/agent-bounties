import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch


PATH = Path(__file__).with_name("verify_open_competition_v2_beta2_broker_reserve.py")
SPEC = importlib.util.spec_from_file_location("verify_open_competition_v2_beta2_broker_reserve", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


def runtime() -> dict:
    return {
        "protocol_version": MODULE.PROTOCOL_VERSION,
        "network": "base-mainnet",
        "settlement_token": "0x" + "44" * 20,
    }


def rpc_result(_url: str, method: str, params: list):
    if method == "eth_chainId":
        return "0x2105"
    if method == "eth_getBlockByNumber":
        return {"number": "0x7b", "hash": "0x" + "aa" * 32}
    if method == "eth_getCode":
        return "0x6000"
    if method == "eth_call":
        return hex(110_000)
    if method == "eth_getBalance":
        return hex(20_000_000_000_000)
    raise AssertionError((method, params))


class BrokerReserveTests(unittest.TestCase):
    @patch.object(MODULE, "rpc", side_effect=rpc_result)
    def test_accepts_exact_isolated_reserves(self, _rpc):
        result = MODULE.inspect_reserve(
            runtime(),
            rpc_url="https://mainnet.example",
            broker="0x" + "11" * 20,
            keeper="0x" + "22" * 20,
            deployer="0x" + "33" * 20,
            minimum_usdc_base_units=110_000,
            minimum_eth_wei=20_000_000_000_000,
        )
        self.assertTrue(result["passed"])
        self.assertTrue(result["roles_are_isolated"])
        self.assertEqual(result["safe_block_number"], 123)

    @patch.object(MODULE, "rpc", side_effect=rpc_result)
    def test_rejects_reused_role_address(self, _rpc):
        with self.assertRaisesRegex(MODULE.BrokerReserveError, "must be distinct"):
            MODULE.inspect_reserve(
                runtime(),
                rpc_url="https://mainnet.example",
                broker="0x" + "11" * 20,
                keeper="0x" + "11" * 20,
                deployer="0x" + "33" * 20,
                minimum_usdc_base_units=110_000,
                minimum_eth_wei=20_000_000_000_000,
            )

    @patch.object(
        MODULE,
        "rpc",
        side_effect=lambda u, m, p: hex(109_999) if m == "eth_call" else rpc_result(u, m, p),
    )
    def test_rejects_underfunded_usdc_reserve(self, _rpc):
        with self.assertRaisesRegex(MODULE.BrokerReserveError, "USDC refund reserve"):
            MODULE.inspect_reserve(
                runtime(),
                rpc_url="https://mainnet.example",
                broker="0x" + "11" * 20,
                keeper="0x" + "22" * 20,
                deployer="0x" + "33" * 20,
                minimum_usdc_base_units=110_000,
                minimum_eth_wei=20_000_000_000_000,
            )


if __name__ == "__main__":
    unittest.main()
