#!/usr/bin/env python3
"""Characterization tests for shared EVM encoding helpers."""

from __future__ import annotations

import unittest

from _shared.evm import address_word, artifact_hex, create_address, keccak256, uint_word


class EvmHelpersTest(unittest.TestCase):
    def test_known_vectors(self) -> None:
        cases = (
            (keccak256(b""), "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"),
            (create_address("0x0000000000000000000000000000000000000000", 0), "0xbd770416a3345f91e4b34576cb804a576fa48eb1"),
            (address_word("0x0000000000000000000000000000000000000001").hex(), "00" * 31 + "01"),
            (uint_word(256).hex(), "00" * 30 + "0100"),
        )
        for actual, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(actual, expected)

    def test_validation_messages_remain_compatible(self) -> None:
        cases = (
            ({"object": "0x1"}, False, "artifact bytecode is missing concrete bytecode"),
            ({"object": "0x1"}, True, "artifact bytecode has odd-length bytecode"),
            ({"object": "0xzz"}, True, "artifact bytecode is missing concrete bytecode"),
        )
        for field, distinct, message in cases:
            with self.subTest(field=field, distinct=distinct):
                with self.assertRaisesRegex(ValueError, f"^{message}$"):
                    artifact_hex(field, "bytecode", distinct_odd_length_error=distinct)

        for value in (-1, 1 << 256):
            with self.subTest(value=value), self.assertRaisesRegex(
                ValueError, "^uint256 is out of range$"
            ):
                uint_word(value)


if __name__ == "__main__":
    unittest.main()
