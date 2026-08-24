#!/usr/bin/env python3
"""Inspect, simulate, and execute the exact confirmed 6-USDC V2 reward cohort."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
from typing import Any

from build_open_competition_v2_reward_policy import (
    CHAIN_ID,
    EXPECTED_LIFETIME_SPENT,
    EXPECTED_RESERVE_BALANCE,
    FUTURE_FLOOR_RESERVE,
    OWNER,
    SCHEMA,
    USDC,
    build_rotation,
)
from local_open_competition_v2_gmv_guard import (
    HASH,
    GuardError,
    code_at,
    code_hash,
    contract_call,
    dual_equal,
    exclusive_guard,
    execute_direct,
    expected_proxy_hash,
    normalized,
    public_address,
    resume_pending,
    rpc,
    safe_block,
)


POLICY_OUTPUTS = [
    "address",
    "uint64",
    "uint64",
    "uint64",
    "uint256",
    "uint256",
    "uint256",
    "uint256",
    "uint256",
    "bytes32",
    "bytes32",
    "bytes32",
]
RESULT_SCHEMA = "agent-bounties/open-competition-v2-reward-execution-v1"
TARGET = 5


class RewardExecutionError(GuardError):
    pass


def validate_reward_accounting(
    policy: dict[str, Any],
    used_count: int,
    period_spent: int,
    lifetime_spent: int,
    balance: int,
) -> int:
    per_competition = int(policy["exact_funding_per_competition"])
    expected_spend = used_count * per_competition
    if (
        lifetime_spent != EXPECTED_LIFETIME_SPENT + expected_spend
        or balance != EXPECTED_RESERVE_BALANCE - expected_spend
        or period_spent < 0
        or period_spent > expected_spend
        or period_spent % per_competition != 0
    ):
        raise RewardExecutionError("live reward spend counters or reserve balance differ")
    remaining = TARGET - used_count
    if (
        remaining < 0
        or period_spent + remaining * per_competition > int(policy["max_per_period"])
        or lifetime_spent + remaining * per_competition
        > int(policy["max_lifetime_spend"])
        or balance - remaining * per_competition < FUTURE_FLOOR_RESERVE
    ):
        raise RewardExecutionError(
            "confirmed policy lacks exact bounded capacity for the treatment"
        )
    return remaining


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(value, dict):
        raise RewardExecutionError(f"{path.name} must contain one object")
    return value


def validate_inputs(
    cohort: dict[str, Any],
    reviewed_state: dict[str, Any],
    bundle: dict[str, Any],
    confirmation: dict[str, Any],
    delegate: str,
) -> None:
    if (
        bundle.get("schema_version") != SCHEMA
        or int(bundle.get("chain_id", 0)) != CHAIN_ID
    ):
        raise RewardExecutionError("reward policy bundle schema or chain is invalid")
    if normalized(bundle.get("owner")) != OWNER:
        raise RewardExecutionError("reward policy owner is invalid")
    if normalized(bundle.get("next_policy", {}).get("delegate")) != delegate:
        raise RewardExecutionError(
            "reward policy is not bound to the protected delegate"
        )
    activation_time = datetime.fromtimestamp(
        int(bundle["next_policy"]["valid_after"]) + 60, tz=timezone.utc
    )
    rebuilt = build_rotation(cohort, reviewed_state, activation_time)
    if rebuilt != bundle:
        raise RewardExecutionError(
            "reward policy is not the exact deterministic reviewed build"
        )
    expected = bundle["next_policy"]
    if (
        confirmation.get("status") != "confirmed"
        or int(confirmation.get("policy_version", -1)) != int(expected["version"])
        or normalized(confirmation.get("policy_hash")) != normalized(expected["hash"])
        or int(confirmation.get("lifetime_spent", -1)) != EXPECTED_LIFETIME_SPENT
        or int(confirmation.get("reserve_balance", -1)) != EXPECTED_RESERVE_BALANCE
        or int(confirmation.get("usdc_moved_by_confirmation", -1)) != 0
        or not HASH.fullmatch(normalized(confirmation.get("revoke_transaction_hash")))
        or not HASH.fullmatch(
            normalized(confirmation.get("configure_transaction_hash"))
        )
    ):
        raise RewardExecutionError("owner policy confirmation evidence is invalid")
    if (
        len(bundle.get("creations", [])) != TARGET
        or len(bundle.get("approved_creation_commitments", [])) != TARGET
    ):
        raise RewardExecutionError(
            "exactly five reviewed reward creations are required"
        )


def inspect_reward_state(
    primary: str,
    shadow: str,
    bundle: dict[str, Any],
    reviewed_state: dict[str, Any],
    *,
    minimum_block: int = 1,
) -> dict[str, Any]:
    safe = safe_block(primary, shadow, minimum_block)
    block = int(safe["number"])
    if (
        int(str(rpc(primary, "eth_chainId", [], 601)), 16) != CHAIN_ID
        or int(str(rpc(shadow, "eth_chainId", [], 602)), 16) != CHAIN_ID
    ):
        raise RewardExecutionError("both inspection RPCs must be Base mainnet")

    reserve = normalized(bundle["reserve_wallet"])
    factory = normalized(reviewed_state["competition_factory"])
    implementation = normalized(reviewed_state["competition_implementation"])
    runtime_checks = (
        (reserve, normalized(reviewed_state["reserve_runtime_code_hash"]), "reserve"),
        (
            USDC,
            normalized(reviewed_state["settlement_token_runtime_code_hash"]),
            "settlement token",
        ),
        (
            factory,
            normalized(reviewed_state["competition_factory_runtime_code_hash"]),
            "competition factory",
        ),
        (
            implementation,
            normalized(reviewed_state["competition_implementation_runtime_code_hash"]),
            "competition implementation",
        ),
    )
    for target, expected_hash, label in runtime_checks:
        observed = dual_equal(
            primary,
            shadow,
            lambda url, target=target: code_hash(code_at(url, target, block)),
        )
        if observed != expected_hash:
            raise RewardExecutionError(
                f"{label} runtime differs from the reviewed state"
            )

    def reserve_view(
        url: str,
        signature: str,
        outputs: list[str],
        inputs: list[str] | None = None,
        values: list[object] | None = None,
    ) -> tuple[Any, ...]:
        return contract_call(url, reserve, signature, outputs, block, inputs, values)

    owner = normalized(
        dual_equal(
            primary, shadow, lambda url: reserve_view(url, "owner()", ["address"])[0]
        )
    )
    token = normalized(
        dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "settlementToken()", ["address"])[0],
        )
    )
    live_factory = normalized(
        dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "competitionFactory()", ["address"])[0],
        )
    )
    version = int(
        dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "policyVersion()", ["uint64"])[0],
        )
    )
    policy_hash = (
        "0x"
        + dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "activePolicyHash()", ["bytes32"])[0],
        ).hex()
    )
    revoked = bool(
        dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "revoked()", ["bool"])[0],
        )
    )
    policy_values = dual_equal(
        primary,
        shadow,
        lambda url: reserve_view(url, "policy()", POLICY_OUTPUTS),
    )
    policy = {
        "delegate": normalized(policy_values[0]),
        "valid_after": int(policy_values[1]),
        "valid_until": int(policy_values[2]),
        "period_seconds": int(policy_values[3]),
        "solver_reward": int(policy_values[4]),
        "keeper_reward": int(policy_values[5]),
        "exact_funding_per_competition": int(policy_values[6]),
        "max_per_period": int(policy_values[7]),
        "max_lifetime_spend": int(policy_values[8]),
        "beta_risk_hash": "0x" + policy_values[9].hex(),
        "gmv_metric_program_hash": "0x" + policy_values[10].hex(),
        "gmv_journal_schema_hash": "0x" + policy_values[11].hex(),
    }
    expected_policy = {key: bundle["next_policy"][key] for key in policy}
    expected_policy["delegate"] = normalized(expected_policy["delegate"])
    for key in (
        "beta_risk_hash",
        "gmv_metric_program_hash",
        "gmv_journal_schema_hash",
    ):
        expected_policy[key] = normalized(expected_policy[key])
    if (
        owner != OWNER
        or token != USDC
        or live_factory != factory
        or version != int(bundle["next_policy"]["version"])
        or policy_hash != normalized(bundle["next_policy"]["hash"])
        or revoked
        or policy != expected_policy
    ):
        raise RewardExecutionError(
            "live reserve policy differs from the confirmed reward policy"
        )

    period_bucket = int(
        dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "periodBucket()", ["uint256"])[0],
        )
    )
    period_spent = int(
        dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "periodSpent()", ["uint256"])[0],
        )
    )
    lifetime_spent = int(
        dual_equal(
            primary,
            shadow,
            lambda url: reserve_view(url, "lifetimeSpent()", ["uint256"])[0],
        )
    )
    balance = int(
        dual_equal(
            primary,
            shadow,
            lambda url: contract_call(
                url,
                USDC,
                "balanceOf(address)",
                ["uint256"],
                block,
                ["address"],
                [reserve],
            )[0],
        )
    )
    current_bucket = int(safe["timestamp"]) // int(policy["period_seconds"])
    effective_period_spent = period_spent if period_bucket == current_bucket else 0

    proxy_hash = expected_proxy_hash(implementation)
    creations: list[dict[str, Any]] = []
    used_count = 0
    active_count = 0
    for creation in bundle["creations"]:
        commitment = normalized(creation["creation_commitment"])
        competition = normalized(creation["predicted_competition"])
        approved = bool(
            dual_equal(
                primary,
                shadow,
                lambda url, commitment=commitment: reserve_view(
                    url,
                    "isApprovedCreation(bytes32)",
                    ["bool"],
                    ["bytes32"],
                    [bytes.fromhex(commitment[2:])],
                )[0],
            )
        )
        used = bool(
            dual_equal(
                primary,
                shadow,
                lambda url, commitment=commitment: reserve_view(
                    url,
                    "usedCreation(bytes32)",
                    ["bool"],
                    ["bytes32"],
                    [bytes.fromhex(commitment[2:])],
                )[0],
            )
        )
        deployed_code = dual_equal(
            primary,
            shadow,
            lambda url, competition=competition: code_at(url, competition, block),
        )
        status = None
        if not approved:
            raise RewardExecutionError(
                "a reviewed creation is not approved by policy v2"
            )
        if used:
            used_count += 1
            if deployed_code == "0x" or code_hash(deployed_code) != proxy_hash:
                raise RewardExecutionError(
                    "a used creation has unexpected deployed code"
                )
            canonical = bool(
                dual_equal(
                    primary,
                    shadow,
                    lambda url, competition=competition: contract_call(
                        url,
                        factory,
                        "isCanonicalCompetition(address)",
                        ["bool"],
                        block,
                        ["address"],
                        [competition],
                    )[0],
                )
            )
            fields = dual_equal(
                primary,
                shadow,
                lambda url, competition=competition: (
                    normalized(
                        contract_call(
                            url, competition, "creator()", ["address"], block
                        )[0]
                    ),
                    normalized(
                        contract_call(
                            url, competition, "settlementToken()", ["address"], block
                        )[0]
                    ),
                    int(
                        contract_call(
                            url, competition, "targetAmount()", ["uint256"], block
                        )[0]
                    ),
                    int(
                        contract_call(
                            url, competition, "fundedAmount()", ["uint256"], block
                        )[0]
                    ),
                    int(
                        contract_call(url, competition, "status()", ["uint8"], block)[0]
                    ),
                ),
            )
            creator, settlement_token, target_amount, funded_amount, status = fields
            if (
                not canonical
                or creator != reserve
                or settlement_token != USDC
                or target_amount != int(policy["exact_funding_per_competition"])
                or funded_amount != int(policy["exact_funding_per_competition"])
                or status not in (1, 2, 3)
            ):
                raise RewardExecutionError(
                    "a used creation is not the exact canonical funded competition"
                )
            if status == 1:
                active_count += 1
        elif deployed_code != "0x":
            raise RewardExecutionError("an unused reviewed creation already has code")
        allowance = int(
            dual_equal(
                primary,
                shadow,
                lambda url, competition=competition: contract_call(
                    url,
                    USDC,
                    "allowance(address,address)",
                    ["uint256"],
                    block,
                    ["address", "address"],
                    [reserve, competition],
                )[0],
            )
        )
        if allowance != 0:
            raise RewardExecutionError("reserve has a nonzero competition allowance")
        creations.append(
            {
                "candidate_id": creation["candidate_id"],
                "commitment": commitment,
                "competition": competition,
                "approved": approved,
                "used": used,
                "status": status,
            }
        )

    remaining = validate_reward_accounting(
        policy,
        used_count,
        effective_period_spent,
        lifetime_spent,
        balance,
    )
    return {
        "safe_block": safe,
        "policy_version": version,
        "policy_hash": policy_hash,
        "period_spent_base_units": effective_period_spent,
        "lifetime_spent_base_units": lifetime_spent,
        "reserve_balance_base_units": balance,
        "used_count": used_count,
        "active_count": active_count,
        "remaining_count": remaining,
        "creations": creations,
    }


def selected_creations(
    state: dict[str, Any], bundle: dict[str, Any]
) -> list[dict[str, Any]]:
    unused = {item["candidate_id"] for item in state["creations"] if not item["used"]}
    return [item for item in bundle["creations"] if item["candidate_id"] in unused]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--cohort", type=Path, required=True)
    parser.add_argument("--reviewed-safe-state", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--confirmation", type=Path, required=True)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--shadow-rpc-url", required=True)
    parser.add_argument("--execution-rpc-url")
    parser.add_argument("--shadow-execution-rpc-url")
    parser.add_argument("--receipt-rpc-url")
    parser.add_argument("--shadow-receipt-rpc-url")
    parser.add_argument("--json-out", type=Path)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("status")
    execute_parser = commands.add_parser("execute")
    execute_parser.add_argument("--broadcast", action="store_true")
    args = parser.parse_args(argv)
    try:
        state_dir = args.state_dir.resolve()
        delegate = public_address(state_dir)
        cohort = load_object(args.cohort)
        reviewed_state = load_object(args.reviewed_safe_state)
        bundle = load_object(args.bundle)
        confirmation = load_object(args.confirmation)
        validate_inputs(cohort, reviewed_state, bundle, confirmation, delegate)
        execution_primary = args.execution_rpc_url or args.rpc_url
        execution_shadow = args.shadow_execution_rpc_url or args.shadow_rpc_url
        receipt_primary = args.receipt_rpc_url or args.rpc_url
        receipt_shadow = args.shadow_receipt_rpc_url or args.shadow_rpc_url
        with exclusive_guard(state_dir):
            resumed = resume_pending(
                state_dir,
                execution_primary,
                execution_shadow,
                receipt_primary,
                receipt_shadow,
            )
            state = inspect_reward_state(
                args.rpc_url,
                args.shadow_rpc_url,
                bundle,
                reviewed_state,
                minimum_block=int(confirmation["transaction_block"]),
            )
            selected = selected_creations(state, bundle)
            result: dict[str, Any] = {
                "schema_version": RESULT_SCHEMA,
                "status": "inspected" if args.command == "status" else "ready",
                "selected_candidate_ids": [item["candidate_id"] for item in selected],
                "state": state,
                "resumed": resumed,
                "executions": [],
                "evidence_boundary": "A dry run or broadcast is not activation evidence. Only exact canonical CompetitionCreatedV2 events plus reconciled funded active contracts establish activation.",
            }
            if args.command == "execute":
                for creation in selected:
                    if args.broadcast:
                        latest = inspect_reward_state(
                            args.rpc_url,
                            args.shadow_rpc_url,
                            bundle,
                            reviewed_state,
                            minimum_block=int(confirmation["transaction_block"]),
                        )
                        latest_selected = selected_creations(latest, bundle)
                        if (
                            not latest_selected
                            or latest_selected[0]["candidate_id"]
                            != creation["candidate_id"]
                        ):
                            raise RewardExecutionError(
                                "canonical state changed after planning; refusing stale creation"
                            )
                    execution = execute_direct(
                        state_dir,
                        execution_primary,
                        execution_shadow,
                        receipt_primary,
                        receipt_shadow,
                        creation["delegate_transaction"],
                        "create_reward_competition",
                        creation["candidate_id"],
                        broadcast=bool(args.broadcast),
                    )
                    result["executions"].append(execution)
                if args.broadcast:
                    minimum = max(
                        [int(item["receipt_block"]) for item in result["executions"]]
                        or [int(confirmation["transaction_block"])]
                    )
                    state = inspect_reward_state(
                        args.rpc_url,
                        args.shadow_rpc_url,
                        bundle,
                        reviewed_state,
                        minimum_block=minimum,
                    )
                    if state["used_count"] != TARGET or state["active_count"] != TARGET:
                        raise RewardExecutionError(
                            "reward execution ended without five exact active competitions"
                        )
                    result["state"] = state
                    result["status"] = "canonically_activated"
                elif selected:
                    result["status"] = "simulated"
                else:
                    result["status"] = "noop"
        if args.json_out:
            args.json_out.parent.mkdir(parents=True, exist_ok=True)
            args.json_out.write_text(
                json.dumps(result, indent=2) + "\n", encoding="utf-8"
            )
        print(
            json.dumps(
                {
                    "status": result["status"],
                    "used": result["state"]["used_count"],
                    "active": result["state"]["active_count"],
                    "selected": result["selected_candidate_ids"],
                    "safe_block": result["state"]["safe_block"],
                },
                sort_keys=True,
            )
        )
        return 0
    except (
        OSError,
        json.JSONDecodeError,
        KeyError,
        TypeError,
        ValueError,
        RewardExecutionError,
    ) as error:
        print(json.dumps({"status": "blocked", "error": str(error)}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
