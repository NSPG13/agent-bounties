import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "deploy_autonomous_v2_mainnet", Path(__file__).resolve().parent / "deploy_autonomous_v2_mainnet.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

FACTORY = "0x" + "fa" * 20
IMPLEMENTATION = "0x" + "1e" * 20


def fake_keccak(value: str) -> str:
    return "0x" + hashlib.sha256(value.encode()).hexdigest()


def fake_run(signer: str):
    def run(*args, **_):
        if args[:3] == ("cast", "wallet", "address"):
            return signer
        if args[:2] == ("cast", "keccak"):
            return fake_keccak(args[2])
        raise AssertionError(f"unexpected command {args[:2]}")
    return run


class FakeChain:
    rpc = "http://rpc.invalid"

    def __init__(self, overrides=None):
        protocol = fake_keccak(MODULE.PROTOCOL_LABEL)
        self.values = {
            (FACTORY, "implementation()(address)"): IMPLEMENTATION,
            (FACTORY, "settlementToken()(address)"): MODULE.USDC,
            (FACTORY, "platformFeeBps()(uint16)"): str(MODULE.FEE_BPS),
            (FACTORY, "platformFeeRecipient()(address)"): MODULE.FEE_RECIPIENT,
            (FACTORY, "SUPPORTED_PROTOCOL_VERSION()(bytes32)"): protocol,
            (IMPLEMENTATION, "protocolVersion()(bytes32)"): protocol,
        }
        self.values.update(overrides or {})

    def call(self, target, signature, *_):
        return self.values[(target, signature)]

    def wait_for_code(self, target):
        return "0x6080" + target[2:6]


class DeployGuardTests(unittest.TestCase):
    def test_refuses_a_key_that_is_not_the_pinned_keeper(self):
        with mock.patch.object(MODULE, "run", fake_run("0x" + "99" * 20)):
            with self.assertRaisesRegex(MODULE.DeploymentError, "not the pinned keeper"):
                MODULE.deploy(FakeChain(), "0x" + "11" * 32)

    def test_refuses_when_a_mainnet_deployment_is_already_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            record = Path(directory) / "deployments" / "autonomous-v2-base-mainnet.json"
            record.parent.mkdir()
            record.write_text("{}")
            with mock.patch.object(MODULE, "run", fake_run(MODULE.KEEPER)), \
                    mock.patch.object(MODULE, "RECORD", record), mock.patch.object(MODULE, "ROOT", Path(directory)):
                with self.assertRaisesRegex(MODULE.DeploymentError, "already records"):
                    MODULE.deploy(FakeChain(), "0x" + "11" * 32)


class VerifyTests(unittest.TestCase):
    def test_accepts_the_launch_terms_and_records_code_hashes(self):
        with mock.patch.object(MODULE, "run", fake_run(MODULE.KEEPER)):
            verified = MODULE.verify(FakeChain(), FACTORY)
        self.assertEqual(verified["platform_fee_bps"], 750)
        self.assertEqual(verified["implementation"], IMPLEMENTATION)
        self.assertTrue(verified["factory_runtime_code_hash"].startswith("0x"))

    def test_rejects_any_term_that_differs(self):
        cases = {
            (FACTORY, "platformFeeBps()(uint16)"): "700",
            (FACTORY, "platformFeeRecipient()(address)"): "0x" + "ab" * 20,
            (FACTORY, "settlementToken()(address)"): "0x" + "cd" * 20,
            (IMPLEMENTATION, "protocolVersion()(bytes32)"): fake_keccak("agent-bounties/autonomous-v1"),
        }
        for key, value in cases.items():
            with self.subTest(key[1]), mock.patch.object(MODULE, "run", fake_run(MODULE.KEEPER)):
                with self.assertRaisesRegex(MODULE.DeploymentError, "disagrees with the launch terms"):
                    MODULE.verify(FakeChain({key: value}), FACTORY)


class ParsingTests(unittest.TestCase):
    def test_reads_forge_json_compact_or_pretty_printed(self):
        pretty = 'Compiling...\n{\n  "deployedTo": "0xAB",\n  "transactionHash": "0xCD"\n}\n'
        self.assertEqual(MODULE.last_json_object(pretty, "deployedTo")["deployedTo"], "0xAB")
        self.assertIsNone(MODULE.last_json_object("no json", "deployedTo"))

    def test_address_extraction_is_strict(self):
        self.assertEqual(MODULE.address("Computed Address: 0x5fb9d8c9CA013C09fA3302B4C93Ac95Da8D34B18", "x"),
                         "0x5fb9d8c9ca013c09fa3302b4c93ac95da8d34b18")
        with self.assertRaises(MODULE.DeploymentError):
            MODULE.address("0x1234", "x")


if __name__ == "__main__":
    unittest.main()
