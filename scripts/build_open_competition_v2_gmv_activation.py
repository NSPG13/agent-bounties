#!/usr/bin/env python3
"""Build the exact owner-signature and delegate-call bundle for the GMV reserve.

The output is deterministic for an injected activation time. It contains no
signature, private key, transaction hash, or claim of on-chain activation.
"""

from __future__ import annotations

import argparse
import json
import re
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from eth_abi import encode
from eth_utils import keccak, to_checksum_address

from build_forward_open_competition_v2_gmv_candidate_pool import predict_reserve_wallet


ROOT = Path(__file__).resolve().parents[1]
CHAIN_ID = 8453
PROTOCOL = "agent-bounties/open-competition-v2-beta3"
PROFILE_ID = "forward-canonical-gmv-attribution-metric-v2"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
DELEGATE = "0xfb58949365e3a30fd62e86edb0daffccf4ef7477"
INITIAL_FUNDING = 77_668_098
SOLVER_REWARD = 3_000_000
KEEPER_REWARD = 40_000
PER_COMPETITION = SOLVER_REWARD + KEEPER_REWARD
DAILY_CAP = 30_400_000
PERIOD_SECONDS = 86_400
PROOF_WINDOW_SECONDS = 90 * 86_400
PARAM_TYPE = "(uint256,uint256,uint64,uint64,uint8,uint8,int256,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32)"
POLICY_TYPE = "(address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32)"
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")


class ActivationError(ValueError):
    pass


def parse_time(value: str, field: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ActivationError(f"{field} must be ISO-8601") from error
    if parsed.tzinfo is None:
        raise ActivationError(f"{field} must include a timezone")
    return parsed.astimezone(timezone.utc)


def address(value: object, field: str) -> str:
    normalized = str(value or "").lower()
    if not ADDRESS.fullmatch(normalized):
        raise ActivationError(f"{field} must be an EVM address")
    return normalized


def hash32(value: object, field: str) -> str:
    normalized = str(value or "").lower()
    if not HASH.fullmatch(normalized) or normalized == "0x" + "00" * 32:
        raise ActivationError(f"{field} must be a nonzero bytes32")
    return normalized


def raw_hash(value: str) -> bytes:
    return bytes.fromhex(value[2:])


def hex_hash(value: bytes) -> str:
    return "0x" + value.hex()


def selector(signature: str) -> bytes:
    return keccak(text=signature)[:4]


def calldata(signature: str, types: list[str], values: list[object]) -> str:
    return "0x" + (selector(signature) + encode(types, values)).hex()


def create2(deployer: str, salt: bytes, init_code_hash: bytes) -> str:
    return "0x" + keccak(b"\xff" + bytes.fromhex(deployer[2:]) + salt + init_code_hash)[12:].hex()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value, dict):
        raise ActivationError(f"{path} must contain a JSON object")
    return value


def reviewed_profile(release: dict[str, Any]) -> dict[str, Any]:
    matches = [
        value
        for value in release.get("metric_programs", [])
        if isinstance(value, dict) and value.get("profile_id") == PROFILE_ID
    ]
    if len(matches) != 1 or matches[0].get("classification") != "reviewed":
        raise ActivationError("the canonical GMV profile is not uniquely reviewed in the release")
    return matches[0]


def build_activation(
    release: dict[str, Any],
    reserve_deployment: dict[str, Any],
    pool: dict[str, Any],
    activation_time: datetime,
    owner: str = OWNER,
    delegate: str = DELEGATE,
) -> dict[str, Any]:
    owner = address(owner, "owner")
    delegate = address(delegate, "delegate")
    if release.get("protocol_version") != PROTOCOL or release.get("network") != "base-mainnet":
        raise ActivationError("release is not the Base mainnet V2 Beta3 protocol")
    if address(release.get("settlement_token"), "settlement token") != USDC:
        raise ActivationError("release does not settle in native Base USDC")
    factory = address(release.get("factory_contract"), "competition factory")
    implementation = address(release.get("implementation_contract"), "competition implementation")
    release_hash = hash32(release.get("release_hash"), "release hash")
    beta_risk_hash = hash32(release.get("beta_risk_hash"), "Beta risk hash")
    profile = reviewed_profile(release)

    canonical = reserve_deployment.get("canonical", {})
    if (
        canonical.get("protocol_version") != PROTOCOL
        or address(canonical.get("competition_factory"), "reserve competition factory") != factory
        or address(canonical.get("settlement_token"), "reserve settlement token") != USDC
        or hash32(canonical.get("release_hash"), "reserve release hash") != release_hash
    ):
        raise ActivationError("reserve deployment is not bound to the exact release")
    reserve_factory = address(
        reserve_deployment.get("reserve_factory", {}).get("address"), "reserve factory"
    )
    reserve_implementation = address(
        reserve_deployment.get("reserve_factory", {}).get("implementation"),
        "reserve implementation",
    )
    reserve_factory_deployment = str(
        reserve_deployment.get("reserve_factory", {}).get("deployment_transaction") or ""
    ).lower()
    if not re.fullmatch(r"0x(?:[0-9a-f]{2})+", reserve_factory_deployment):
        raise ActivationError("reserve factory deployment transaction is invalid")
    user_salt = keccak(
        text=f"agent-bounties/base-mainnet/gmv-meta-reserve/{owner}/{release_hash}/v1"
    )
    reserve = predict_reserve_wallet(
        reserve_factory, reserve_implementation, release_hash, owner
    )

    if (
        pool.get("schema_version")
        != "agent-bounties/open-competition-v2-forward-gmv-meta-candidate-specs-v2"
        or pool.get("protocol_version") != PROTOCOL
        or pool.get("network") != "base-mainnet"
        or address(pool.get("factory_contract"), "candidate factory") != factory
        or hash32(pool.get("release_hash"), "candidate release hash") != release_hash
        or address(pool.get("reserve_wallet"), "candidate reserve wallet") != reserve
    ):
        raise ActivationError("candidate pool is not bound to the exact release")
    approved_at = parse_time(str(pool.get("approved_at")), "candidate approval time")
    expires_at = parse_time(str(pool.get("expires_at")), "candidate approval expiry")
    if not approved_at <= activation_time < expires_at or approved_at >= expires_at:
        raise ActivationError("candidate approval window is not current")
    pool_profile = pool.get("profile_release")
    if (
        not isinstance(pool_profile, dict)
        or pool_profile.get("profile_id") != PROFILE_ID
        or pool_profile.get("status") != "reviewed"
    ):
        raise ActivationError("candidate pool does not use the canonical GMV profile")
    excluded_wallets = [
        address(value, "candidate excluded wallet")
        for value in pool.get("eligibility_policy", {}).get("excluded_wallets", [])
    ]
    if excluded_wallets != sorted(excluded_wallets) or reserve not in excluded_wallets:
        raise ActivationError("candidate pool does not exclude its exact reserve wallet")
    for field in (
        "program_vkey",
        "source_hash",
        "elf_hash",
        "journal_schema_hash",
        "metric_program_hash",
    ):
        if hash32(pool_profile.get(field), f"candidate {field}") != hash32(profile.get(field), f"release {field}"):
            raise ActivationError(f"candidate {field} differs from the reviewed release")
    execution_policy_hash = hash32(pool_profile.get("execution_policy_hash"), "execution policy")
    settlement_policy_hash = hash32(pool_profile.get("settlement_policy_hash"), "settlement policy")
    proof_system = hex_hash(keccak(text="sp1-plonk"))

    candidates = pool.get("candidates")
    if not isinstance(candidates, list) or len(candidates) != 20:
        raise ActivationError("candidate pool must contain exactly twenty candidates")
    funding_deadline = int(expires_at.timestamp())
    creations: list[dict[str, Any]] = []
    commitments: list[bytes] = []
    ids: set[str] = set()
    for index, candidate in enumerate(candidates):
        if not isinstance(candidate, dict):
            raise ActivationError(f"candidate {index} is invalid")
        candidate_id = str(candidate.get("candidate_id") or "")
        if not candidate_id or candidate_id in ids:
            raise ActivationError("candidate IDs must be nonempty and unique")
        ids.add(candidate_id)
        if candidate.get("gmv_lane") != "external_supply":
            raise ActivationError(f"{candidate_id} does not target external demand")
        snapshot = candidate.get("snapshot")
        epoch = candidate.get("epoch")
        if not isinstance(snapshot, dict) or snapshot.get("status") != "scheduled" or not isinstance(epoch, dict):
            raise ActivationError(f"{candidate_id} lacks a scheduled forward campaign")
        starts_at = parse_time(str(epoch.get("starts_at")), f"{candidate_id} starts_at")
        ends_at = parse_time(str(epoch.get("ends_at")), f"{candidate_id} ends_at")
        if starts_at >= ends_at or ends_at >= expires_at:
            raise ActivationError(f"{candidate_id} forward campaign timing is invalid")
        score_threshold = int(epoch.get("minimum_score_base_units", 0))
        if score_threshold <= 0:
            raise ActivationError(f"{candidate_id} score threshold is invalid")
        verification_policy_hash = hash32(
            snapshot.get("verification_policy_hash"), f"{candidate_id} verification policy"
        )
        params = (
            SOLVER_REWARD,
            KEEPER_REWARD,
            funding_deadline,
            PROOF_WINDOW_SECONDS,
            1,
            0,
            score_threshold,
            raw_hash(proof_system),
            raw_hash(hash32(profile.get("program_vkey"), "program vkey")),
            raw_hash(hash32(profile.get("source_hash"), "source hash")),
            raw_hash(hash32(profile.get("elf_hash"), "ELF hash")),
            raw_hash(hash32(profile.get("journal_schema_hash"), "journal schema")),
            raw_hash(hash32(profile.get("metric_program_hash"), "metric program")),
            raw_hash(execution_policy_hash),
            raw_hash(verification_policy_hash),
            raw_hash(settlement_policy_hash),
            raw_hash(beta_risk_hash),
        )
        creation_nonce = keccak(
            text=f"agent-bounties/base-mainnet/gmv-meta/{release_hash}/{candidate_id}/v1"
        )
        commitment = keccak(
            encode(
                ["uint256", "address", PARAM_TYPE, "bytes32"],
                [CHAIN_ID, to_checksum_address(factory), params, creation_nonce],
            )
        )
        commitments.append(commitment)
        creations.append(
            {
                "candidate_id": candidate_id,
                "title": candidate.get("title"),
                "summary": candidate.get("summary"),
                "creation_nonce": hex_hash(creation_nonce),
                "creation_commitment": hex_hash(commitment),
                "params": {
                    "solver_reward": SOLVER_REWARD,
                    "keeper_reward": KEEPER_REWARD,
                    "funding_deadline": funding_deadline,
                    "proof_window_seconds": PROOF_WINDOW_SECONDS,
                    "winner_mode": "best_score",
                    "score_direction": "higher_is_better",
                    "score_threshold": score_threshold,
                    "proof_system": proof_system,
                    "program_vkey": hash32(profile.get("program_vkey"), "program vkey"),
                    "source_hash": hash32(profile.get("source_hash"), "source hash"),
                    "elf_hash": hash32(profile.get("elf_hash"), "ELF hash"),
                    "journal_schema_hash": hash32(profile.get("journal_schema_hash"), "journal schema"),
                    "metric_program_hash": hash32(profile.get("metric_program_hash"), "metric program"),
                    "execution_policy_hash": execution_policy_hash,
                    "verification_policy_hash": verification_policy_hash,
                    "settlement_policy_hash": settlement_policy_hash,
                    "beta_risk_hash": beta_risk_hash,
                },
                "_params_abi": params,
            }
        )

    valid_after = max(0, int(activation_time.timestamp()) - 60)
    valid_until = int(expires_at.timestamp())
    policy = (
        to_checksum_address(delegate),
        valid_after,
        valid_until,
        PERIOD_SECONDS,
        SOLVER_REWARD,
        KEEPER_REWARD,
        PER_COMPETITION,
        DAILY_CAP,
        INITIAL_FUNDING,
        raw_hash(beta_risk_hash),
        raw_hash(hash32(profile.get("metric_program_hash"), "metric program")),
        raw_hash(hash32(profile.get("journal_schema_hash"), "journal schema")),
    )
    policy_hash = keccak(encode([POLICY_TYPE, "bytes32[]"], [policy, commitments]))
    effective_salt = keccak(
        encode(
            ["address", "bytes32"],
            [to_checksum_address(owner), user_salt],
        )
    )
    clone_init = bytes.fromhex("3d602d80600a3d3981f3") + bytes.fromhex(
        "363d3d373d3d3d363d73"
    ) + bytes.fromhex(reserve_implementation[2:]) + bytes.fromhex(
        "5af43d82803e903d91602b57fd5bf3"
    )
    if reserve != create2(reserve_factory, effective_salt, keccak(clone_init)):
        raise ActivationError("independent reserve prediction disagrees")

    competition_init = bytes.fromhex("3d602d80600a3d3981f3") + bytes.fromhex(
        "363d3d373d3d3d363d73"
    ) + bytes.fromhex(implementation[2:]) + bytes.fromhex(
        "5af43d82803e903d91602b57fd5bf3"
    )
    for creation in creations:
        params = creation.pop("_params_abi")
        nonce = raw_hash(creation["creation_nonce"])
        bounty_id = keccak(
            encode(
                ["uint256", "address", "address", "bytes32", PARAM_TYPE],
                [CHAIN_ID, to_checksum_address(factory), to_checksum_address(reserve), nonce, params],
            )
        )
        predicted = create2(factory, bounty_id, keccak(competition_init))
        creation["bounty_id"] = hex_hash(bounty_id)
        creation["predicted_competition"] = predicted
        creation["delegate_transaction"] = {
            "from": delegate,
            "to": reserve,
            "value_wei": 0,
            "data": calldata(
                f"createCompetition({PARAM_TYPE},bytes32)",
                [PARAM_TYPE, "bytes32"],
                [params, nonce],
            ),
        }

    authorization_valid_before = int((activation_time + timedelta(hours=1)).timestamp())
    authorization_nonce = keccak(
        text=f"agent-bounties/base-mainnet/gmv-meta-reserve-funding/{reserve}/{release_hash}/v1"
    )
    typed_data = {
        "types": {
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"},
            ],
            "TransferWithAuthorization": [
                {"name": "from", "type": "address"},
                {"name": "to", "type": "address"},
                {"name": "value", "type": "uint256"},
                {"name": "validAfter", "type": "uint256"},
                {"name": "validBefore", "type": "uint256"},
                {"name": "nonce", "type": "bytes32"},
            ],
        },
        "primaryType": "TransferWithAuthorization",
        "domain": {
            "name": "USD Coin",
            "version": "2",
            "chainId": CHAIN_ID,
            "verifyingContract": to_checksum_address(USDC),
        },
        "message": {
            "from": to_checksum_address(owner),
            "to": to_checksum_address(reserve),
            "value": str(INITIAL_FUNDING),
            "validAfter": "0",
            "validBefore": str(authorization_valid_before),
            "nonce": hex_hash(authorization_nonce),
        },
    }
    policy_json = {
        "delegate": delegate,
        "valid_after": valid_after,
        "valid_until": valid_until,
        "period_seconds": PERIOD_SECONDS,
        "solver_reward": SOLVER_REWARD,
        "keeper_reward": KEEPER_REWARD,
        "exact_funding_per_competition": PER_COMPETITION,
        "max_per_period": DAILY_CAP,
        "max_lifetime_spend": INITIAL_FUNDING,
        "beta_risk_hash": beta_risk_hash,
        "gmv_metric_program_hash": hash32(profile.get("metric_program_hash"), "metric program"),
        "gmv_journal_schema_hash": hash32(profile.get("journal_schema_hash"), "journal schema"),
    }
    return {
        "schema_version": "agent-bounties/open-competition-v2-forward-gmv-meta-activation-v2",
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "protocol_version": PROTOCOL,
        "release_hash": release_hash,
        "competition_factory": factory,
        "reserve_factory": reserve_factory,
        "reserve_wallet": reserve,
        "owner": owner,
        "delegate": delegate,
        "initial_funding_base_units": INITIAL_FUNDING,
        "initial_funding_usdc": "77.668098",
        "policy": policy_json,
        "policy_hash": hex_hash(policy_hash),
        "user_salt": hex_hash(user_salt),
        "approved_creation_commitments": [hex_hash(value) for value in commitments],
        "creations": creations,
        "reserve_factory_deployment": {
            "to": reserve_deployment["deterministic_deployer"]["address"],
            "value_wei": 0,
            "data": reserve_factory_deployment,
            "predicted_reserve_factory": reserve_factory,
        },
        "owner_authorization": {
            "kind": "eip3009_transfer_with_authorization",
            "typed_data": typed_data,
            "valid_before": authorization_valid_before,
            "relay_function": f"createWalletWithAuthorization(address,{POLICY_TYPE},bytes32[],bytes32,uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)",
            "unsigned": True,
        },
        "recovery": {
            "owner": owner,
            "revoke_policy_call": {"to": reserve, "data": calldata("revokePolicy()", [], [])},
            "recover_uncommitted_call": {
                "to": reserve,
                "data": calldata("recoverUncommitted()", [], []),
            },
            "boundary": "The owner can revoke and recover uncommitted USDC. Active competition escrow follows its canonical settlement or refund path.",
        },
        "confirmation_summary": {
            "wallet": owner,
            "token": USDC,
            "amount": "77.668098 USDC",
            "destination": reserve,
            "destination_kind": "owner-recoverable bounded Open Competition V2 reserve",
            "maximum_single_competition": "3.04 USDC",
            "maximum_utc_day": "30.40 USDC",
            "maximum_lifetime": "77.668098 USDC",
            "initial_active_target": 10,
            "approved_candidate_count": 20,
            "objective": "highest externally funded canonical marketplace GMV",
        },
        "evidence_boundary": "This unsigned bundle is not deployment, funding, activation, GMV, payout, or settlement evidence.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--reserve-deployment", type=Path, required=True)
    parser.add_argument(
        "--candidate-pool",
        type=Path,
        default=ROOT / "ops" / "open-competition-v2-forward-gmv-candidate-pool-v2.json",
    )
    parser.add_argument("--owner", default=OWNER)
    parser.add_argument("--delegate", default=DELEGATE)
    parser.add_argument("--activation-time")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        now = (
            parse_time(args.activation_time, "activation time")
            if args.activation_time
            else datetime.now(timezone.utc)
        )
        bundle = build_activation(
            load_json(args.release),
            load_json(args.reserve_deployment),
            load_json(args.candidate_pool),
            now,
            args.owner,
            args.delegate,
        )
    except (OSError, json.JSONDecodeError, ActivationError, ValueError) as error:
        print(f"activation build blocked: {error}")
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
