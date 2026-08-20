import unittest

from eth_account import Account

import recover_open_competition_v2_rehearsal_funds as recovery


class RehearsalRecoveryTests(unittest.TestCase):
    def test_actor_derivation_is_stable_and_unique(self) -> None:
        root = bytes.fromhex("11" * 32)
        actors = recovery.actor_set(
            root,
            "4d09d82825c38f2bf93a8ee4375a95b302410c29",
            "32147289466",
            [1, 2, 4],
        )
        self.assertEqual(len(actors), 6)
        self.assertEqual(len({actor.address.lower() for actor in actors}), 6)
        self.assertEqual(Account.from_key(root).address, "0x19E7E376E7C213B7E7e7e46cc70A5dD086DAff2A")

    def test_eth_sweep_keeps_a_conservative_gas_reserve(self) -> None:
        self.assertEqual(recovery.sweepable_eth(400_000, 2), 100_000)
        self.assertEqual(recovery.sweepable_eth(299_999, 2), 0)

    def test_additional_actor_scope_is_explicit_and_validated(self) -> None:
        scope = recovery.parse_actor_scope(
            "b1ce5c176a3cf6d4e3d4240b0a2fdf1359a3a8c5:32122203861:1"
        )
        self.assertEqual(
            scope,
            ("b1ce5c176a3cf6d4e3d4240b0a2fdf1359a3a8c5", "32122203861", 1),
        )
        with self.assertRaises(recovery.RecoveryError):
            recovery.parse_actor_scope("main:32122203861:1")

    def test_transfer_calldata_binds_recipient_and_amount(self) -> None:
        data = recovery.transfer_data("0x" + "12" * 20, 525_000)
        self.assertTrue(data.startswith("0xa9059cbb"))
        self.assertEqual(len(data), 138)
        self.assertTrue(data.endswith((525_000).to_bytes(32, "big").hex()))

    def test_refund_calldata_binds_the_contributor(self) -> None:
        contributor = "0x" + "34" * 20
        data = recovery.address_call_data("withdrawRefundFor(address)", contributor)
        self.assertEqual(len(data), 74)
        self.assertTrue(data.endswith("34" * 20))

    def test_expired_active_competition_without_leader_requires_expiry(self) -> None:
        self.assertTrue(
            recovery.expired_competition_needs_expiry(
                status=1,
                proof_deadline=100,
                block_timestamp=101,
                leader=recovery.ZERO_ADDRESS,
            )
        )

    def test_cancelled_expired_competition_is_idempotent(self) -> None:
        self.assertFalse(
            recovery.expired_competition_needs_expiry(
                status=3,
                proof_deadline=0,
                block_timestamp=0,
                leader=recovery.ZERO_ADDRESS,
            )
        )

    def test_expired_recovery_rejects_leader_and_early_deadline(self) -> None:
        with self.assertRaisesRegex(recovery.RecoveryError, "deadline has not passed"):
            recovery.expired_competition_needs_expiry(1, 100, 100, recovery.ZERO_ADDRESS)
        with self.assertRaisesRegex(recovery.RecoveryError, "qualifying leader"):
            recovery.expired_competition_needs_expiry(1, 100, 101, "0x" + "12" * 20)


if __name__ == "__main__":
    unittest.main()
