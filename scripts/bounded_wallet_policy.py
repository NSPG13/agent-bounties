#!/usr/bin/env python3
"""Exact active Base-mainnet bounded-wallet policy approved by its owner."""

WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662"
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
DELEGATE = "0xe46741de0f379bff0ab8b01bce1b79a12d892fdb"
POLICY_HASH = "0xe865752db0df29aa0fc682fa837b7a68b91d0c88272cd6e0ae6718c831ada959"
POLICY_VERSION = 6
CONFIGURATION_TRANSACTION = (
    "0x09532bbf5382cadac12c14c010cf332a7082d3e7ff018362e13991c5dfbb5704"
)
CONFIGURATION_BLOCK = 49_902_575
VALID_AFTER = 1_784_223_027
VALID_UNTIL = (1 << 64) - 1
PERIOD_SECONDS = 86_400
MAX_PER_ACTION = 5_000_000
MAX_PER_PERIOD = 10_000_000
MAX_LIFETIME_SPEND = 89_000_000
MAX_BOUNTY_TARGET = 5_000_000
ALLOWED_ACTIONS = 15
ALLOWED_VERIFICATION_MODES = 3
DETERMINISTIC_VERIFIER = "0x380c1af742593dd88b6f20387e9ee693a0536731"
SIGNED_QUORUM_VERIFIER_SET_HASH = (
    "0x2c5a10915ca1fb99d4a11e2222b4f32b986b4e0f5599f55d70e9c8f9725a28cd"
)
AI_JUDGE_VERIFIER_SET_HASH = "0x" + "00" * 32


def expected_state() -> dict[str, object]:
    return {
        "owner": OWNER,
        "delegate": DELEGATE,
        "valid_after": VALID_AFTER,
        "valid_until": VALID_UNTIL,
        "period_seconds": PERIOD_SECONDS,
        "max_per_action": MAX_PER_ACTION,
        "max_per_period": MAX_PER_PERIOD,
        "max_lifetime_spend": MAX_LIFETIME_SPEND,
        "max_bounty_target": MAX_BOUNTY_TARGET,
        "allowed_actions": ALLOWED_ACTIONS,
        "allowed_verification_modes": ALLOWED_VERIFICATION_MODES,
        "deterministic_verifier": DETERMINISTIC_VERIFIER,
        "signed_quorum": SIGNED_QUORUM_VERIFIER_SET_HASH,
        "ai_quorum": AI_JUDGE_VERIFIER_SET_HASH,
        "policy_hash": POLICY_HASH,
        "policy_version": POLICY_VERSION,
    }
