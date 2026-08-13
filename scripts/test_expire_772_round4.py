import unittest
from unittest.mock import patch

import scripts.expire_772_round4 as module


class FakeCast:
    def safe_block(self):
        return {"number": "0x64", "hash": "0x" + "aa" * 32, "timestamp": hex(module.VERIFICATION_EXPIRES_AT + 1)}

    def run(self, *args):
        if args == ("chain-id",): return str(module.CHAIN_ID)
        if args[:2] == ("codehash", module.CONTRACT): return module.CLONE_CODEHASH
        if args[:3] == ("call", module.CONTRACT, module.SELECTOR): return "0x"
        if args[:3] == ("estimate", module.CONTRACT, module.SELECTOR): return "80000"
        raise AssertionError(args)

    def call(self, target, signature, *args, block):
        values = {
            "bountyId()(bytes32)": module.BOUNTY_ID,
            "status()(uint8)": "3",
            "round()(uint64)": "4",
            "solver()(address)": module.SOLVER,
            "verificationExpiresAt()(uint64)": str(module.VERIFICATION_EXPIRES_AT),
            "activeClaimBond()(uint256)": str(module.BOND),
            "balanceOf(address)(uint256)": "123456",
        }
        return values[signature]


class Expiry772Tests(unittest.TestCase):
    def test_dry_run_is_exact_and_non_executing(self):
        report = module.dry_run(FakeCast())
        self.assertEqual(report["intent"]["data"], "0xf9251ec7")
        self.assertFalse(report["execution"]["performed"])
        self.assertEqual(report["tuple"]["round"], 4)

    def test_each_tuple_field_fails_closed(self):
        fields = {
            "codehash": "0x" + "11" * 32,
            "bounty_id": "0x" + "22" * 32,
            "status": "2",
            "round": "5",
            "solver": "0x" + "33" * 20,
            "verificationExpiresAt": str(module.VERIFICATION_EXPIRES_AT + 1),
            "activeClaimBond": "9999",
        }
        for field, bad in fields.items():
            cast = FakeCast()
            if field == "codehash":
                original = cast.run
                cast.run = lambda *args, _original=original: bad if args[:2] == ("codehash", module.CONTRACT) else _original(*args)
            elif field == "bounty_id":
                original = cast.call
                cast.call = lambda target, sig, *args, block, _original=original: bad if sig == "bountyId()(bytes32)" else _original(target, sig, *args, block=block)
            else:
                signature = {"verificationExpiresAt": "verificationExpiresAt()(uint64)", "activeClaimBond": "activeClaimBond()(uint256)"}.get(field, f"{field}()({'address' if field == 'solver' else 'uint8' if field == 'status' else 'uint64'})")
                original = cast.call
                cast.call = lambda target, sig, *args, block, _original=original, _signature=signature: bad if sig == _signature else _original(target, sig, *args, block=block)
            with self.subTest(field=field), self.assertRaises(module.RecoveryError):
                module.snapshot(cast)

    def test_execute_requires_exact_ack_before_network(self):
        with self.assertRaisesRegex(module.RecoveryError, "acknowledge"):
            module.execute_once(FakeCast(), "wrong")

    def test_facade_constants_cannot_be_overridden_by_cli(self):
        self.assertEqual(module.CONTRACT, "0x9baa8a4a2ad3096c6ebfb2c994a93afb7a299274")
        self.assertEqual(module.SELECTOR, "0xf9251ec7")
        self.assertEqual(module.CHAIN_ID, 8453)


if __name__ == "__main__":
    unittest.main()
