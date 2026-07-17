#!/usr/bin/env python3

from __future__ import annotations

import json
import unittest
from dataclasses import replace
from unittest import mock

import relay_bounded_wallet_action as relay
from plan_bounded_agent_budget import POLICY_TYPE, calldata, encode, keccak_hex, policy_tuple


NOW = 1_800_000_000
BOUNTY_ID = "0x" + "aa" * 32
PREDICTED = "0x4444444444444444444444444444444444444444"
PAYLOAD_HASH = "0x" + "bb" * 32
SIGNATURE = "0x" + "cc" * 65
KEEPER = "0x5555555555555555555555555555555555555555"


def create_data() -> str:
    params = (
        "(2000000,10000,"
        + "0x" + "11" * 32 + ","
        + "0x" + "22" * 32 + ","
        + "0x" + "33" * 32 + ","
        + "0x" + "44" * 32 + ","
        + "0x" + "55" * 32 + f",{NOW + 86_400},604800,259200,0,"
        + relay.VERIFIER
        + ",0x8888888888888888888888888888888888888888,1)"
    )
    return calldata(
        relay.decode_create_calldata.__globals__["CREATE_SIGNATURE"],
        params,
        "[]",
        "2010000",
        "0x" + "99" * 32,
    )


def signed_create_data(*, reverse_verifiers: bool = False) -> str:
    params = (
        "(2000000,100000,"
        + "0x" + "11" * 32 + ","
        + "0x" + "22" * 32 + ","
        + "0x" + "33" * 32 + ","
        + "0x" + "44" * 32 + ","
        + "0x" + "55" * 32 + f",{NOW + 86_400},604800,259200,1,"
        + relay.ZERO_ADDRESS
        + ","
        + relay.ZERO_ADDRESS
        + ",2)"
    )
    verifiers = list(relay.SIGNED_QUORUM_VERIFIERS)
    if reverse_verifiers:
        verifiers.reverse()
    return calldata(
        relay.decode_create_calldata.__globals__["CREATE_SIGNATURE"],
        params,
        f"[{','.join(verifiers)}]",
        "2100000",
        "0x" + "98" * 32,
    )


def envelope(**overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "schema": relay.SCHEMA,
        "network": relay.NETWORK,
        "action": "create",
        "issue_number": 249,
        "wallet": relay.WALLET,
        "policy_hash": relay.POLICY_HASH,
        "policy_version": 2,
        "nonce": 0,
        "deadline": NOW + 600,
        "payload": "0x" + create_data()[10:],
        "payload_hash": PAYLOAD_HASH,
        "signature": SIGNATURE,
        "bounty_id": BOUNTY_ID,
        "predicted_bounty_contract": PREDICTED,
    }
    value.update(overrides)
    return value


def wallet_state(**overrides: object) -> relay.WalletState:
    values: dict[str, object] = {
        "chain_id": relay.CHAIN_ID,
        "block_timestamp": NOW,
        "codehash": relay.WALLET_CODEHASH,
        "registered": True,
        "factory": relay.FACTORY,
        "settlement_token": relay.USDC,
        "deployment_factory": relay.WALLET_FACTORY,
        "owner": "0x884834e884d6e93462655a2820140ad03e6747bc",
        "delegate": relay.DELEGATE,
        "valid_after": NOW - 10,
        "valid_until": NOW + 86_400,
        "period_seconds": 86_400,
        "max_per_action": 5_000_000,
        "max_per_period": 10_000_000,
        "max_lifetime_spend": 89_000_000,
        "max_bounty_target": 5_000_000,
        "allowed_actions": 15,
        "allowed_verification_modes": 1,
        "deterministic_verifier": relay.VERIFIER,
        "signed_quorum_verifier_set_hash": relay.ZERO_HASH,
        "policy_hash": relay.POLICY_HASH,
        "policy_version": 2,
        "delegate_nonce": 0,
        "period_bucket": NOW // 86_400,
        "period_spent": 0,
        "lifetime_spent": 0,
        "wallet_usdc_balance": 89_000_000,
        "wallet_eth_balance": 0,
        "revoked": False,
    }
    values.update(overrides)
    return relay.WalletState(**values)  # type: ignore[arg-type]


class ValidationClient:
    rpc_url = "https://example.test"

    def __init__(self, *, signature_valid: bool = True) -> None:
        self.signature_valid = signature_valid

    def run(self, *args: str, retry: bool = True) -> str:
        if args[0] == "keccak":
            return PAYLOAD_HASH
        if args[0] == "code":
            return "0x"
        if args[:2] == ("wallet", "verify"):
            if not self.signature_valid:
                raise relay.RelayError("invalid signature")
            return "Validation succeeded"
        raise AssertionError(f"unexpected run {args}")

    def call(self, contract: str, signature: str, *args: str, block: str | None = None) -> str:
        if signature == relay.BOUNTY_ID_SIGNATURE:
            return BOUNTY_ID
        if signature == relay.PREDICT_SIGNATURE:
            return PREDICTED
        if signature == "isCanonicalBounty(address)(bool)":
            return "false"
        if signature.startswith("actionDigest("):
            return "0x" + "dd" * 32
        raise AssertionError(f"unexpected call {signature}")


class RelayClient:
    def estimate(self, keeper: str, contract: str, signature: str, *args: str) -> int:
        return 500_000

    def gas_price(self) -> int:
        return 1_000_000

    def send(self, private_key: str, gas_limit: int, contract: str, signature: str, *args: str) -> dict:
        return {
            "status": "0x1",
            "to": contract,
            "transactionHash": "0x" + "ef" * 32,
            "blockNumber": "0x123",
        }

    def call(self, contract: str, signature: str, *args: str, block: str | None = None) -> str:
        if signature == "allowance(address,address)(uint256)":
            return "0"
        raise AssertionError(f"unexpected call {signature}")


class BoundedWalletRelayTests(unittest.TestCase):
    def test_regression_policy_hash_matches_exact_owner_review(self) -> None:
        policy = {
            "delegate": relay.REGRESSION_DELEGATE,
            "valid_after": 1784223027,
            "valid_until": 1786815027,
            "period_seconds": 86_400,
            "max_per_action": 5_000_000,
            "max_per_period": 10_000_000,
            "max_lifetime_spend": 89_000_000,
            "max_bounty_target": 5_000_000,
            "allowed_actions": 15,
            "allowed_verification_modes": 3,
            "deterministic_verifier_module": relay.VERIFIER,
            "signed_quorum_verifier_set_hash": relay.SIGNED_QUORUM_VERIFIER_SET_HASH,
            "ai_judge_verifier_set_hash": relay.ZERO_HASH,
        }
        encoded = encode(f"f({POLICY_TYPE})", policy_tuple(policy))
        self.assertEqual(keccak_hex(encoded), relay.REGRESSION_POLICY_HASH)

    def test_comment_requires_exact_command_and_schema(self) -> None:
        body = relay.COMMAND + "\n```json\n" + json.dumps(envelope()) + "\n```"
        self.assertEqual(relay.parse_comment(body)["issue_number"], 249)
        with self.assertRaisesRegex(relay.RelayError, "first line"):
            relay.parse_comment("please relay\n{}")
        with self.assertRaisesRegex(relay.RelayError, "keys mismatch"):
            relay.parse_comment(relay.COMMAND + "\n" + json.dumps(envelope(extra=True)))

    def test_event_requires_funding_label_and_matching_issue(self) -> None:
        source = relay.RelaySource(relay.REPOSITORY, 249, 1, "agent", ("bounty",))
        body = relay.COMMAND + "\n" + json.dumps(envelope())
        with self.assertRaisesRegex(relay.RelayError, "funding-needed"):
            relay.parse_event_request(source, body)
        source = replace(source, labels=("bounty", "funding-needed"))
        with self.assertRaisesRegex(relay.RelayError, "does not match"):
            relay.parse_event_request(source, relay.COMMAND + "\n" + json.dumps(envelope(issue_number=250)))

    def test_exact_signed_creation_passes_without_agent_eth(self) -> None:
        state = wallet_state()
        relay.validate_wallet(state, envelope())
        prepared = relay.validate_signed_creation(ValidationClient(), envelope(), state)
        self.assertEqual(prepared["initial_funding"], 2_010_000)
        self.assertEqual(state.wallet_eth_balance, 0)

    def test_exact_regression_creation_passes_and_other_quorums_fail(self) -> None:
        signed_envelope = envelope(
            policy_hash=relay.REGRESSION_POLICY_HASH,
            policy_version=3,
            payload="0x" + signed_create_data()[10:],
        )
        state = wallet_state(
            delegate=relay.REGRESSION_DELEGATE,
            allowed_verification_modes=3,
            signed_quorum_verifier_set_hash=relay.SIGNED_QUORUM_VERIFIER_SET_HASH,
            policy_hash=relay.REGRESSION_POLICY_HASH,
            policy_version=3,
        )
        relay.validate_wallet(state, signed_envelope)
        prepared = relay.validate_signed_creation(ValidationClient(), signed_envelope, state)
        self.assertEqual(prepared["initial_funding"], 2_100_000)

        reversed_envelope = {
            **signed_envelope,
            "payload": "0x" + signed_create_data(reverse_verifiers=True)[10:],
        }
        with self.assertRaisesRegex(relay.RelayError, "exact signed regression"):
            relay.validate_signed_creation(ValidationClient(), reversed_envelope, state)

    def test_signature_and_budget_fail_closed(self) -> None:
        with self.assertRaisesRegex(relay.RelayError, "invalid signature"):
            relay.validate_signed_creation(ValidationClient(signature_valid=False), envelope(), wallet_state())
        with self.assertRaisesRegex(relay.RelayError, "remaining bounded-wallet budget"):
            relay.validate_signed_creation(
                ValidationClient(), envelope(), wallet_state(period_spent=9_000_000)
            )

    def test_relay_reports_keeper_gas_and_zero_agent_eth(self) -> None:
        before = wallet_state()
        after = wallet_state(
            delegate_nonce=1,
            period_spent=2_010_000,
            lifetime_spent=2_010_000,
            wallet_usdc_balance=86_990_000,
        )
        prepared = {
            "decoded": relay.decode_create_calldata(create_data()),
            "target": 2_010_000,
            "initial_funding": 2_010_000,
            "bounty_id": BOUNTY_ID,
            "predicted": PREDICTED,
            "payload_hash": PAYLOAD_HASH,
        }
        event = relay.RelayEvent(
            relay.REPOSITORY,
            249,
            1,
            "agent",
            ("bounty", "funding-needed"),
            envelope(),
        )
        with (
            mock.patch.object(relay, "preflight_keeper", return_value=("0x" + "01" * 32, KEEPER, 10**18)),
            mock.patch.object(relay, "read_wallet_state", side_effect=[before, after]),
            mock.patch.object(relay, "validate_wallet"),
            mock.patch.object(relay, "validate_signed_creation", return_value=prepared),
            mock.patch.object(relay, "validate_receipt", return_value=("0x" + "ef" * 32, "0x123")),
            mock.patch.object(relay, "validate_created_bounty", return_value={"canonical": True}),
        ):
            report = relay.relay(
                RelayClient(),
                event,
                execute=True,
                private_key="0x" + "01" * 32,
            )
        self.assertEqual(report["outcome"], "relayed")
        self.assertEqual(report["gas_payer"], KEEPER)
        self.assertEqual(report["agent_wallet_eth_required_wei"], 0)
        self.assertEqual(report["agent_wallet_eth_spent_wei"], 0)

    def test_replay_reconciles_without_keeper_or_second_broadcast(self) -> None:
        current = wallet_state(delegate_nonce=1, lifetime_spent=2_010_000, wallet_usdc_balance=86_990_000)
        prepared = {
            "decoded": relay.decode_create_calldata(create_data()),
            "target": 2_010_000,
            "initial_funding": 2_010_000,
            "bounty_id": BOUNTY_ID,
            "predicted": PREDICTED,
            "payload_hash": PAYLOAD_HASH,
        }
        event = relay.RelayEvent(
            relay.REPOSITORY,
            249,
            2,
            "agent",
            ("bounty", "funding-needed"),
            envelope(deadline=NOW - 1),
        )
        with (
            mock.patch.object(relay, "read_wallet_state", return_value=current),
            mock.patch.object(relay, "validate_signed_creation", return_value=prepared) as validate,
            mock.patch.object(relay, "validate_created_bounty", return_value={"canonical": True}),
            mock.patch.object(relay, "preflight_keeper") as preflight,
        ):
            report = relay.relay(RelayClient(), event, execute=True, private_key=None)
        self.assertEqual(report["outcome"], "already-relayed")
        self.assertTrue(report["claimable"])
        validate.assert_called_once_with(
            mock.ANY,
            event.envelope,
            current,
            allow_existing=True,
        )
        preflight.assert_not_called()


if __name__ == "__main__":
    unittest.main()
