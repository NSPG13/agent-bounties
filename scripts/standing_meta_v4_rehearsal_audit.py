#!/usr/bin/env python3
"""Audit complete Standing Meta V4 Base Sepolia rehearsal evidence through RPC."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
from typing import Any, Mapping


DEPLOY_SCRIPT = Path(__file__).with_name("standing_meta_v4_deploy.py")
DEPLOY_SPEC = importlib.util.spec_from_file_location("standing_meta_v4_deploy", DEPLOY_SCRIPT)
assert DEPLOY_SPEC and DEPLOY_SPEC.loader
DEPLOY = importlib.util.module_from_spec(DEPLOY_SPEC)
DEPLOY_SPEC.loader.exec_module(DEPLOY)

SCHEMA = "agent-bounties/standing-meta-v4-sepolia-rehearsal-v1"
AUDIT_SCHEMA = "agent-bounties/standing-meta-v4-sepolia-rehearsal-audit-v1"
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")
GIT_COMMIT_RE = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")

EVENT_SIGNATURES = {
    "PrimaryAssigned": "PrimaryAssigned(bytes32,address,uint64,uint8)",
    "PrimaryAvailabilityFailed": "PrimaryAvailabilityFailed(bytes32,address,uint8)",
    "PrimaryVerdictSubmitted": "PrimaryVerdictSubmitted(bytes32,address,bool,bytes32,uint64)",
    "AppealOpened": "AppealOpened(bytes32,address,uint256,uint256)",
    "AppealJuryAssigned": "AppealJuryAssigned(bytes32,address[],uint64)",
    "VerificationFinalized": "VerificationFinalized(bytes32,bool,bool,bool)",
    "VerificationTimedOut": "VerificationTimedOut(bytes32,bytes32)",
    "SubmissionRejected": "SubmissionRejected(bytes32,uint64,address,uint256,uint256,bytes32)",
    "BountySettled": (
        "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)"
    ),
    "BountyCancelled": "BountyCancelled(bytes32,uint256)",
    "RefundWithdrawn": "RefundWithdrawn(bytes32,address,uint256)",
    "SolutionCommitted": "SolutionCommitted(bytes32,address,uint8,bytes32,uint64,uint64,uint256)",
    "SolutionRevealed": "SolutionRevealed(bytes32,uint64,address,bytes32,bytes32,bool,bytes32)",
}
TRANSFER_EVENT_SIGNATURE = "Transfer(address,address,uint256)"
OPEN_COMPETITION_FACTORY_SOURCE = (
    "src/OpenCompetitionBountyFactoryV1.sol:OpenCompetitionBountyFactoryV1"
)
OPEN_COMPETITION_IMPLEMENTATION_SOURCE = "src/OpenCompetitionBountyV1.sol:OpenCompetitionBountyV1"
V4_CANONICAL_SOURCES = {
    "anonymous_protocol_controller": "src/AnonymousProtocolControllerV1.sol:AnonymousProtocolControllerV1",
    "anonymous_stake_pool": "src/AnonymousStakePoolV1.sol:AnonymousStakePoolV1",
    "verifier_sortition": "src/VrfSortitionCoordinatorV1.sol:VrfSortitionCoordinatorV1",
    "solver_sortition": "src/VrfSortitionCoordinatorV1.sol:VrfSortitionCoordinatorV1",
    "appealable_verifier": "src/AppealableVerifierV1.sol:AppealableVerifierV1",
    "standing_meta_child_factory": "src/StandingMetaChildFactoryV4.sol:StandingMetaChildFactoryV4",
    "standing_meta_parent_factory": "src/StandingMetaParentFactoryV4.sol:StandingMetaParentFactoryV4",
    "onchain_terms_registry": "src/OnchainTermsRegistryV4.sol:OnchainTermsRegistryV4",
    "canonical_independent_child_verifier": (
        "src/CanonicalIndependentChildVerifierV4.sol:CanonicalIndependentChildVerifierV4"
    ),
    "standing_meta_v4_bundle": "src/StandingMetaV4Bundle.sol:StandingMetaV4Bundle",
}
BASE_CHILD_FACTORY_SOURCE = "src/AgentBountyFactory.sol:AgentBountyFactory"

VERIFIER_EVENTS = {
    "PrimaryAssigned",
    "PrimaryAvailabilityFailed",
    "PrimaryVerdictSubmitted",
    "AppealOpened",
    "AppealJuryAssigned",
    "VerificationFinalized",
    "VerificationTimedOut",
}
SUBJECT_EVENTS = set(EVENT_SIGNATURES) - VERIFIER_EVENTS

REQUIRED_SCENARIOS: dict[str, dict[str, Any]] = {
    "unappealed_acceptance": {
        "subject_kinds": {"standing_meta_child"},
        "events": {"PrimaryVerdictSubmitted", "VerificationFinalized", "BountySettled"},
        "facts": {"primary_verdict": True, "appealed": False, "final_verdict": True},
    },
    "primary_rejection": {
        "subject_kinds": {"standing_meta_child"},
        "events": {"PrimaryVerdictSubmitted", "VerificationFinalized", "SubmissionRejected"},
        "facts": {"primary_verdict": False, "appealed": False, "final_verdict": False},
    },
    "solver_appeal_overturned_rejection": {
        "subject_kinds": {"standing_meta_child"},
        "events": {
            "PrimaryVerdictSubmitted",
            "AppealOpened",
            "AppealJuryAssigned",
            "VerificationFinalized",
            "BountySettled",
        },
        "facts": {
            "appellant_role": "solver",
            "primary_verdict": False,
            "final_verdict": True,
            "appealed": True,
            "overturned": True,
            "appeal_bond_micro_usdc": 100_000,
        },
    },
    "creator_appeal_overturned_acceptance": {
        "subject_kinds": {"standing_meta_child"},
        "events": {
            "PrimaryVerdictSubmitted",
            "AppealOpened",
            "AppealJuryAssigned",
            "VerificationFinalized",
            "SubmissionRejected",
        },
        "facts": {
            "appellant_role": "creator",
            "primary_verdict": True,
            "final_verdict": False,
            "appealed": True,
            "overturned": True,
            "appeal_bond_micro_usdc": 100_000,
        },
    },
    "upheld_appeal": {
        "subject_kinds": {"standing_meta_child"},
        "events": {"AppealOpened", "AppealJuryAssigned", "VerificationFinalized"},
        "facts": {"appealed": True, "overturned": False, "appeal_bond_micro_usdc": 100_000},
    },
    "primary_timeout_promotion": {
        "subject_kinds": {"standing_meta_child"},
        "events": {"PrimaryAvailabilityFailed", "PrimaryAssigned"},
        "facts": {"promoted": True, "no_reroll": True},
    },
    "appeal_timeout": {
        "subject_kinds": {"standing_meta_child"},
        "events": {"VerificationTimedOut"},
        "facts": {"appeal_bond_closed": True, "remaining_stake_unlocked": True},
    },
    "cancellation": {
        "subject_kinds": {"standing_meta_child", "standing_meta_parent"},
        "events": {"BountyCancelled"},
        "facts": {"owner_authorized": True},
    },
    "contributor_pull_refund": {
        "subject_kinds": {"standing_meta_child", "standing_meta_parent"},
        "events": {"RefundWithdrawn"},
        "facts": {"pull_refund": True},
    },
    "standing_meta_child_settlement": {
        "subject_kinds": {"standing_meta_child"},
        "events": {"BountySettled"},
        "facts": {"canonical_bounty_settled": True, "solver_reward_micro_usdc": 990_000},
    },
    "standing_meta_parent_settlement": {
        "subject_kinds": {"standing_meta_parent"},
        "events": {"BountySettled"},
        "facts": {
            "canonical_bounty_settled": True,
            "solver_reward_micro_usdc": 2_000_000,
            "maximum_child_outlay_micro_usdc": 1_000_000,
            "successful_settlement_margin_micro_usdc": 1_000_000,
        },
    },
    "open_competition_first_valid_settlement": {
        "subject_kinds": {"open_competition"},
        "events": {"SolutionCommitted", "SolutionRevealed", "BountySettled"},
        "facts": {
            "first_valid_reveal_won": True,
            "commit_reveal_block_separation_minimum": 1,
            "canonical_bounty_settled": True,
        },
    },
}


class RehearsalAuditError(RuntimeError):
    pass


def read_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise RehearsalAuditError(f"expected a JSON object in {path}")
    return value


def write_object(path: Path, value: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def is_address(value: object) -> bool:
    return isinstance(value, str) and bool(ADDRESS_RE.fullmatch(value)) and int(value[2:], 16) != 0


def is_bytes32(value: object) -> bool:
    return isinstance(value, str) and bool(BYTES32_RE.fullmatch(value))


def _uint(value: object) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    if isinstance(value, str):
        try:
            return int(value, 0)
        except ValueError:
            return None
    return None


def content_sha256(evidence: Mapping[str, Any]) -> str:
    """Hash the canonical evidence object with its commitment field omitted."""
    payload = dict(evidence)
    payload.pop("content_sha256", None)
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def _scenario_structure(name: str, value: object) -> tuple[dict[str, bool], list[str]]:
    checks: dict[str, bool] = {}
    blockers: list[str] = []
    required = REQUIRED_SCENARIOS[name]
    if not isinstance(value, Mapping):
        return {"object": False}, [f"scenario {name} is missing or malformed"]
    checks["object"] = True
    checks["confirmed_status"] = value.get("status") == "confirmed"
    checks["scenario_id"] = is_bytes32(value.get("scenario_id"))
    checks["subject_kind"] = value.get("subject_kind") in required["subject_kinds"]
    checks["subject_contract"] = is_address(value.get("subject_contract"))
    checks["bounty_id"] = is_bytes32(value.get("bounty_id"))
    events = value.get("events")
    needs_case_id = isinstance(events, list) and any(
        isinstance(event, Mapping) and event.get("name") in VERIFIER_EVENTS for event in events
    )
    checks["case_id"] = not needs_case_id or is_bytes32(value.get("case_id"))
    actors = value.get("actors")
    checks["actors"] = (
        isinstance(actors, Mapping)
        and is_address(actors.get("creator"))
        and is_address(actors.get("solver"))
        and str(actors["creator"]).lower() != str(actors["solver"]).lower()
    )
    checks["linked_child_contract"] = (
        is_address(value.get("linked_child_contract"))
        if name == "standing_meta_parent_settlement"
        else "linked_child_contract" not in value or is_address(value.get("linked_child_contract"))
    )
    transactions = value.get("transactions")
    checks["transactions"] = (
        isinstance(transactions, list)
        and len(transactions) > 0
        and all(is_bytes32(item) for item in transactions)
        and len({str(item).lower() for item in transactions}) == len(transactions)
    )
    valid_events = isinstance(events, list) and len(events) > 0
    event_names: set[str] = set()
    if valid_events:
        for event in events:
            if not isinstance(event, Mapping):
                valid_events = False
                continue
            event_name = event.get("name")
            event_names.add(str(event_name))
            valid_events = valid_events and (
                event_name in EVENT_SIGNATURES
                and event.get("signature") == EVENT_SIGNATURES.get(event_name)
                and is_address(event.get("contract"))
                and is_bytes32(event.get("transaction_hash"))
                and isinstance(transactions, list)
                and event.get("transaction_hash") in transactions
                and isinstance(event.get("log_index"), int)
                and event["log_index"] >= 0
                and isinstance(event.get("block_number"), int)
                and event["block_number"] > 0
            )
    checks["event_records"] = bool(valid_events)
    checks["required_events"] = required["events"].issubset(event_names)
    facts = value.get("facts")
    checks["required_facts"] = isinstance(facts, Mapping) and all(
        facts.get(key) == expected for key, expected in required["facts"].items()
    )
    for check, passed in checks.items():
        if not passed:
            blockers.append(f"scenario {name} failed {check}")
    return checks, blockers


def audit_structure(evidence: Mapping[str, Any]) -> dict[str, Any]:
    checks = {
        "schema": evidence.get("schema") == SCHEMA,
        "content_sha256": evidence.get("content_sha256") == content_sha256(evidence),
        "source_commit": isinstance(evidence.get("source_commit"), str)
        and bool(GIT_COMMIT_RE.fullmatch(str(evidence["source_commit"]).lower())),
        "network": evidence.get("network") == "base-sepolia",
        "chain_id": evidence.get("chain_id") == DEPLOY.BASE_SEPOLIA_CHAIN_ID,
        "canonical_usdc": str(evidence.get("settlement_token", "")).lower() == DEPLOY.BASE_SEPOLIA_USDC,
        "open_competition_factory": is_address(evidence.get("open_competition_factory")),
        "minimum_native_subscription_reserve": isinstance(
            evidence.get("minimum_native_subscription_reserve_wei"), int
        )
        and evidence["minimum_native_subscription_reserve_wei"] > 0,
        "minimum_gas_sponsorship_reserve": isinstance(
            evidence.get("minimum_gas_sponsorship_reserve_wei"), int
        )
        and evidence["minimum_gas_sponsorship_reserve_wei"] > 0,
    }
    funding = evidence.get("faucet_funding")
    checks["faucet_funding"] = (
        isinstance(funding, Mapping)
        and is_bytes32(funding.get("native_eth_transaction"))
        and isinstance(funding.get("test_usdc_transactions"), list)
        and len(funding["test_usdc_transactions"]) > 0
        and all(is_bytes32(item) for item in funding["test_usdc_transactions"])
        and isinstance(funding.get("confirmed_native_eth_wei"), int)
        and funding["confirmed_native_eth_wei"] > 0
        and isinstance(funding.get("confirmed_test_usdc_base_units"), int)
        and funding["confirmed_test_usdc_base_units"] >= 55_000_000
    )
    observations = evidence.get("rpc_observations")
    checks["independent_rpc_observations"] = (
        isinstance(observations, list)
        and len(observations) == 2
        and {item.get("endpoint_label") for item in observations if isinstance(item, Mapping)}
        == {"primary", "secondary"}
        and all(
            isinstance(item, Mapping)
            and item.get("chain_id") == DEPLOY.BASE_SEPOLIA_CHAIN_ID
            and isinstance(item.get("block_number"), int)
            and item["block_number"] > 0
            for item in observations
        )
    )
    scenarios = evidence.get("scenarios")
    scenario_checks: dict[str, Any] = {}
    blockers = [f"rehearsal structure failed {name}" for name, passed in checks.items() if not passed]
    for name in REQUIRED_SCENARIOS:
        value = scenarios.get(name) if isinstance(scenarios, Mapping) else None
        result, scenario_blockers = _scenario_structure(name, value)
        scenario_checks[name] = result
        blockers.extend(scenario_blockers)
    scenario_ids = (
        [item.get("scenario_id") for item in scenarios.values() if isinstance(item, Mapping)]
        if isinstance(scenarios, Mapping)
        else []
    )
    unique_scenarios = (
        len(scenario_ids) == len(REQUIRED_SCENARIOS)
        and len(set(scenario_ids)) == len(REQUIRED_SCENARIOS)
        and set(scenarios) == set(REQUIRED_SCENARIOS)
    ) if isinstance(scenarios, Mapping) else False
    checks["exact_unique_scenario_set"] = unique_scenarios
    if not unique_scenarios:
        blockers.append("rehearsal scenarios are not the exact unique required set")
    event_keys: list[tuple[str, int]] = []
    if isinstance(scenarios, Mapping):
        for scenario in scenarios.values():
            if not isinstance(scenario, Mapping) or not isinstance(scenario.get("events"), list):
                continue
            for event in scenario["events"]:
                if isinstance(event, Mapping) and is_bytes32(event.get("transaction_hash")):
                    log_index = event.get("log_index")
                    if isinstance(log_index, int):
                        event_keys.append((str(event["transaction_hash"]).lower(), log_index))
    checks["globally_unique_event_records"] = len(event_keys) > 0 and len(set(event_keys)) == len(event_keys)
    if not checks["globally_unique_event_records"]:
        blockers.append("rehearsal event records are missing or reused across scenarios")
    return {
        "checks": checks,
        "scenario_checks": scenario_checks,
        "structure_complete": not blockers,
        "blockers": blockers,
    }


def _receipt_log(receipt: Mapping[str, Any], log_index: int) -> Mapping[str, Any] | None:
    for item in receipt.get("logs", []):
        if not isinstance(item, Mapping):
            continue
        observed = _uint(item.get("logIndex"))
        if observed == log_index:
            return item
    return None


def _data_words(log: Mapping[str, Any]) -> list[int]:
    raw = str(log.get("data", ""))
    if not raw.startswith("0x") or len(raw) < 2 or (len(raw) - 2) % 64:
        raise RehearsalAuditError("event data is not canonical ABI words")
    body = raw[2:]
    if not re.fullmatch(r"[0-9a-fA-F]*", body):
        raise RehearsalAuditError("event data is not hex")
    return [int(body[index : index + 64], 16) for index in range(0, len(body), 64)]


def _topics(log: Mapping[str, Any]) -> list[str]:
    values = log.get("topics")
    if not isinstance(values, list) or not all(is_bytes32(item) for item in values):
        raise RehearsalAuditError("event topics are malformed")
    return [str(item).lower() for item in values]


def _topic_address(topics: list[str], index: int) -> str:
    if index >= len(topics) or int(topics[index][2:26], 16) != 0:
        raise RehearsalAuditError("indexed address topic is malformed")
    return "0x" + topics[index][-40:]


def _word_bool(words: list[int], index: int, label: str) -> bool:
    if index >= len(words) or words[index] not in {0, 1}:
        raise RehearsalAuditError(f"{label} is not an ABI boolean")
    return words[index] == 1


def _call_address(foundry: Any, address: str, signature: str, label: str) -> str:
    return DEPLOY.normalize_address(foundry.call(address, signature), label)


def _call_bytes32(foundry: Any, address: str, signature: str, label: str) -> str:
    return DEPLOY.require_bytes32(foundry.call(address, signature), label)


def _call_bool(foundry: Any, address: str, signature: str, *args: str) -> bool:
    value = foundry.call(address, signature, *args).strip().lower()
    if value in {"true", "1", "0x1", "0x01"}:
        return True
    if value in {"false", "0", "0x0", "0x00"}:
        return False
    raise RehearsalAuditError(f"{signature} did not return a boolean")


def _call_uint(foundry: Any, address: str, signature: str, *args: str) -> int:
    value = _uint(foundry.call(address, signature, *args).strip())
    if value is None or value < 0:
        raise RehearsalAuditError(f"{signature} did not return an unsigned integer")
    return value


def _runtime_matches_compiled_with_immutables(foundry: Any, address: str, source: str) -> bool:
    """Compare runtime code after masking compiler-declared immutable slots."""
    foundry.runtime_hash(source)  # Builds or refreshes the local artifact first.
    source_path, contract_name = source.split(":", 1)
    artifact_path = (
        Path(__file__).resolve().parents[1]
        / "contracts"
        / "base-escrow"
        / "out"
        / Path(source_path).name
        / f"{contract_name}.json"
    )
    artifact = read_object(artifact_path)
    deployed = artifact.get("deployedBytecode")
    if not isinstance(deployed, Mapping):
        raise RehearsalAuditError(f"compiled artifact has no deployed bytecode: {artifact_path}")
    compiled_hex = str(deployed.get("object", "")).removeprefix("0x")
    observed_hex = str(foundry.code(address)).removeprefix("0x")
    if (
        not compiled_hex
        or len(compiled_hex) != len(observed_hex)
        or len(compiled_hex) % 2
        or not re.fullmatch(r"[0-9a-fA-F]+", compiled_hex)
        or not re.fullmatch(r"[0-9a-fA-F]+", observed_hex)
    ):
        return False
    compiled = bytearray.fromhex(compiled_hex)
    observed = bytearray.fromhex(observed_hex)
    references = deployed.get("immutableReferences", {})
    if not isinstance(references, Mapping):
        raise RehearsalAuditError("compiled immutable references are malformed")
    for items in references.values():
        if not isinstance(items, list):
            raise RehearsalAuditError("compiled immutable reference list is malformed")
        for item in items:
            if not isinstance(item, Mapping):
                raise RehearsalAuditError("compiled immutable reference is malformed")
            start = _uint(item.get("start"))
            length = _uint(item.get("length"))
            if start is None or length is None or start < 0 or length <= 0 or start + length > len(compiled):
                raise RehearsalAuditError("compiled immutable reference is out of bounds")
            compiled[start : start + length] = bytes(length)
            observed[start : start + length] = bytes(length)
    return foundry.keccak_text("0x" + compiled.hex()) == foundry.keccak_text("0x" + observed.hex())


def _parse_address_array(value: str) -> list[str]:
    text = value.strip()
    if not (text.startswith("[") and text.endswith("]")):
        raise RehearsalAuditError("eligibleWallets returned a malformed array")
    body = text[1:-1].strip()
    if not body:
        return []
    return [DEPLOY.normalize_address(item.strip(), "eligible wallet") for item in body.split(",")]


def _subject_provenance(
    foundry: Any,
    scenario: Mapping[str, Any],
    canonical: Mapping[str, str],
    evidence: Mapping[str, Any],
) -> dict[str, Any]:
    subject = DEPLOY.normalize_address(scenario["subject_contract"], "scenario subject")
    kind = str(scenario["subject_kind"])
    actors = scenario["actors"]
    checks = {
        "runtime_code": foundry.code(subject) not in {"0x", "0x0"},
        "bounty_id": _call_bytes32(foundry, subject, "bountyId()(bytes32)", "subject bounty id")
        == str(scenario["bounty_id"]).lower(),
        "creator": _call_address(foundry, subject, "creator()(address)", "subject creator")
        == str(actors["creator"]).lower(),
        "settlement_token": _call_address(
            foundry, subject, "settlementToken()(address)", "subject settlement token"
        )
        == DEPLOY.BASE_SEPOLIA_USDC,
    }
    observed_factory = _call_address(foundry, subject, "factory()(address)", "subject factory")
    if kind == "standing_meta_child":
        checks["canonical_factory"] = observed_factory == canonical["standing_meta_child_factory"]
        checks["canonical_verifier"] = _call_address(
            foundry, subject, "verifierModule()(address)", "child verifier"
        ) == canonical["appealable_verifier"]
    elif kind == "standing_meta_parent":
        checks["canonical_factory"] = observed_factory == canonical["standing_meta_parent_factory"]
        checks["canonical_verifier"] = _call_address(
            foundry, subject, "verifierModule()(address)", "parent verifier"
        ) == canonical["canonical_independent_child_verifier"]
        if "linked_child_contract" in scenario:
            child = DEPLOY.normalize_address(scenario["linked_child_contract"], "linked child")
            checks["linked_child_matches_parent"] = _call_address(
                foundry, subject, "preparedChild()(address)", "parent prepared child"
            ) == child
            checks["linked_child_runtime_code"] = foundry.code(child) not in {"0x", "0x0"}
            checks["linked_child_factory"] = _call_address(
                foundry, child, "factory()(address)", "linked child factory"
            ) == canonical["standing_meta_child_factory"]
            checks["linked_child_token"] = _call_address(
                foundry, child, "settlementToken()(address)", "linked child token"
            ) == DEPLOY.BASE_SEPOLIA_USDC
            checks["linked_child_target"] = _call_uint(
                foundry, child, "targetAmount()(uint256)"
            ) == 1_000_000
            checks["linked_child_settled"] = _call_uint(foundry, child, "status()(uint8)") == 4
    elif kind == "open_competition":
        competition_factory = DEPLOY.normalize_address(
            evidence["open_competition_factory"], "open competition factory"
        )
        checks["canonical_factory"] = observed_factory == competition_factory
        checks["factory_runtime_code"] = foundry.code(competition_factory) not in {"0x", "0x0"}
        checks["factory_runtime_hash"] = _runtime_matches_compiled_with_immutables(
            foundry, competition_factory, OPEN_COMPETITION_FACTORY_SOURCE
        )
        checks["factory_settlement_token"] = _call_address(
            foundry, competition_factory, "settlementToken()(address)", "competition factory token"
        ) == DEPLOY.BASE_SEPOLIA_USDC
        checks["factory_marks_canonical"] = _call_bool(
            foundry, competition_factory, "isCanonicalCompetition(address)(bool)", subject
        )
        implementation = _call_address(
            foundry, competition_factory, "implementation()(address)", "competition implementation"
        )
        checks["implementation_runtime_code"] = foundry.code(implementation) not in {"0x", "0x0"}
        checks["implementation_runtime_hash"] = (
            foundry.keccak_text(foundry.code(implementation))
            == foundry.runtime_hash(OPEN_COMPETITION_IMPLEMENTATION_SOURCE)
        )
        verifier = _call_address(foundry, subject, "verifierModule()(address)", "competition verifier")
        checks["verifier_runtime_code"] = foundry.code(verifier) not in {"0x", "0x0"}
    else:
        raise RehearsalAuditError(f"unsupported subject kind: {kind}")
    return {"checks": checks, "complete": all(checks.values()), "subject": subject, "kind": kind}


def _has_usdc_transfer(
    foundry: Any,
    receipt: Mapping[str, Any],
    sender: str,
    recipient: str,
    amount: int,
) -> bool:
    topic0 = foundry.keccak_text(TRANSFER_EVENT_SIGNATURE)
    for log in receipt.get("logs", []):
        if not isinstance(log, Mapping) or str(log.get("address", "")).lower() != DEPLOY.BASE_SEPOLIA_USDC:
            continue
        try:
            topics = _topics(log)
            words = _data_words(log)
            if (
                len(topics) == 3
                and topics[0] == topic0
                and _topic_address(topics, 1) == sender.lower()
                and _topic_address(topics, 2) == recipient.lower()
                and len(words) == 1
                and words[0] == amount
            ):
                return True
        except RehearsalAuditError:
            continue
    return False


def _scenario_fact_checks(
    foundry: Any,
    name: str,
    scenario: Mapping[str, Any],
    records: list[dict[str, Any]],
) -> dict[str, bool]:
    facts = scenario["facts"]
    actors = {key: str(value).lower() for key, value in scenario["actors"].items()}
    subject = str(scenario["subject_contract"]).lower()
    by_name: dict[str, list[dict[str, Any]]] = {}
    for record in records:
        by_name.setdefault(record["event"]["name"], []).append(record)
    checks: dict[str, bool] = {
        "exactly_one_of_each_required_event": all(
            len(by_name.get(event_name, [])) == 1 for event_name in REQUIRED_SCENARIOS[name]["events"]
        )
    }

    def one(event_name: str) -> dict[str, Any] | None:
        items = by_name.get(event_name, [])
        return items[0] if len(items) == 1 else None

    primary = one("PrimaryVerdictSubmitted")
    if "primary_verdict" in facts:
        checks["decoded_primary_verdict"] = primary is not None and _word_bool(
            _data_words(primary["log"]), 0, "primary verdict"
        ) is facts["primary_verdict"]

    finalized = one("VerificationFinalized")
    if any(key in facts for key in ("final_verdict", "appealed", "overturned")):
        if finalized is None:
            checks["decoded_final_verdict"] = False
        else:
            words = _data_words(finalized["log"])
            if "final_verdict" in facts:
                checks["decoded_final_verdict"] = _word_bool(words, 0, "final verdict") is facts["final_verdict"]
            if "appealed" in facts:
                checks["decoded_appealed"] = _word_bool(words, 1, "appealed") is facts["appealed"]
            if "overturned" in facts:
                checks["decoded_overturned"] = _word_bool(words, 2, "overturned") is facts["overturned"]
        if finalized is not None:
            checks["finalized_case_state"] = _call_uint(
                foundry,
                str(finalized["event"]["contract"]),
                "caseState(bytes32)(uint8)",
                str(scenario["case_id"]),
            ) == 6

    appeal = one("AppealOpened")
    if "appeal_bond_micro_usdc" in facts:
        checks["decoded_appeal_bond"] = appeal is not None and len(_data_words(appeal["log"])) >= 2 and (
            _data_words(appeal["log"])[1] == facts["appeal_bond_micro_usdc"]
        )
    if "appellant_role" in facts:
        role = str(facts["appellant_role"])
        checks["decoded_appellant_role"] = (
            appeal is not None
            and role in actors
            and _topic_address(_topics(appeal["log"]), 2) == actors[role]
        )

    for event_name, topic_index in (
        ("SubmissionRejected", 3),
        ("BountySettled", 3),
        ("SolutionCommitted", 2),
        ("SolutionRevealed", 3),
    ):
        record = one(event_name)
        if record is not None:
            checks[f"decoded_{event_name}_solver"] = _topic_address(
                _topics(record["log"]), topic_index
            ) == actors["solver"]

    settled = one("BountySettled")
    if settled is not None:
        words = _data_words(settled["log"])
        reward_matches = "solver_reward_micro_usdc" not in facts or (
            len(words) >= 1 and words[0] == facts["solver_reward_micro_usdc"]
        )
        checks["decoded_solver_reward"] = reward_matches
        checks["canonical_solver_transfer"] = len(words) >= 3 and _has_usdc_transfer(
            foundry,
            settled["receipt"],
            subject,
            actors["solver"],
            words[0] + words[1] + words[2],
        )
        if name == "standing_meta_parent_settlement":
            child_outlay = int(facts["maximum_child_outlay_micro_usdc"])
            checks["decoded_successful_settlement_margin"] = (
                words[0] - child_outlay == facts["successful_settlement_margin_micro_usdc"]
                and child_outlay == 1_000_000
            )

    refund = one("RefundWithdrawn")
    if refund is not None:
        words = _data_words(refund["log"])
        topics = _topics(refund["log"])
        contributor = _topic_address(topics, 2)
        checks["canonical_refund_transfer"] = len(words) == 1 and _has_usdc_transfer(
            foundry, refund["receipt"], subject, contributor, words[0]
        )

    if name == "primary_timeout_promotion":
        failed = one("PrimaryAvailabilityFailed")
        promoted = one("PrimaryAssigned")
        checks["rank_promoted_without_reusing_wallet"] = (
            failed is not None
            and promoted is not None
            and _data_words(failed["log"])[0] < _data_words(promoted["log"])[1]
            and _topic_address(_topics(failed["log"]), 2)
            != _topic_address(_topics(promoted["log"]), 2)
        )

    timed_out = one("VerificationTimedOut")
    if timed_out is not None:
        checks["timed_out_case_state"] = _call_uint(
            foundry,
            str(timed_out["event"]["contract"]),
            "caseState(bytes32)(uint8)",
            str(scenario["case_id"]),
        ) == 7

    if name == "open_competition_first_valid_settlement":
        committed = one("SolutionCommitted")
        revealed = one("SolutionRevealed")
        checks["first_valid_ordering"] = False
        if committed is not None and revealed is not None and settled is not None:
            commit_words = _data_words(committed["log"])
            reveal_words = _data_words(revealed["log"])
            reveal_topics = _topics(revealed["log"])
            settle_topics = _topics(settled["log"])
            minimum = int(facts["commit_reveal_block_separation_minimum"])
            checks["first_valid_ordering"] = (
                len(commit_words) >= 2
                and len(reveal_words) >= 3
                and _word_bool(reveal_words, 2, "competition reveal verdict")
                and revealed["block_number"] >= commit_words[1] + minimum
                and reveal_topics[2] == settle_topics[2]
                and revealed["event"]["transaction_hash"].lower()
                == settled["event"]["transaction_hash"].lower()
            )
    return checks


def audit_live_pass(
    foundry: Any,
    evidence: Mapping[str, Any],
    deployment: Mapping[str, Any],
    endpoint_label: str,
) -> dict[str, Any]:
    if foundry.chain_id() != DEPLOY.BASE_SEPOLIA_CHAIN_ID:
        raise RehearsalAuditError("live rehearsal audit requires Base Sepolia")
    deployment_verification = DEPLOY.verify_deployment(foundry, deployment)
    canonical = deployment_verification["canonical_component_addresses"]
    runtime_source_checks = {
        name: _runtime_matches_compiled_with_immutables(foundry, canonical[name], source)
        for name, source in V4_CANONICAL_SOURCES.items()
    }
    base_child_factory = deployment_verification["dependency_addresses"]["base_child_factory"]
    runtime_source_checks["base_child_factory"] = _runtime_matches_compiled_with_immutables(
        foundry, base_child_factory, BASE_CHILD_FACTORY_SOURCE
    )
    pool = canonical["anonymous_stake_pool"]
    eligible_solvers = _parse_address_array(
        foundry.call(pool, "eligibleWallets(uint8,address[])(address[])", "0", "[]")
    )
    eligible_verifiers = _parse_address_array(
        foundry.call(pool, "eligibleWallets(uint8,address[])(address[])", "1", "[]")
    )
    subscription = deployment_verification["subscription"]
    minimum_subscription = int(evidence["minimum_native_subscription_reserve_wei"])
    minimum_gas = int(evidence["minimum_gas_sponsorship_reserve_wei"])
    keeper = DEPLOY.normalize_address(deployment["deployer"], "deployer")
    keeper_balance = foundry.balance(keeper)
    local_commit = DEPLOY.run(["git", "rev-parse", "HEAD"], cwd=foundry.repo).strip().lower()
    worktree_status = DEPLOY.run(
        ["git", "status", "--porcelain", "--untracked-files=normal"], cwd=foundry.repo
    ).strip()
    observation_block = _uint(foundry.rpc("block-number"))
    if observation_block is None or observation_block <= 0:
        raise RehearsalAuditError(f"{endpoint_label} RPC returned an invalid block number")
    declared_observation = next(
        item
        for item in evidence["rpc_observations"]
        if item["endpoint_label"] == endpoint_label
    )

    receipt_cache: dict[str, Mapping[str, Any]] = {}

    def receipt(transaction_hash: str) -> Mapping[str, Any]:
        key = transaction_hash.lower()
        if key not in receipt_cache:
            receipt_cache[key] = foundry.receipt(key)
        return receipt_cache[key]

    event_checks: list[dict[str, Any]] = []
    scenario_checks: dict[str, Any] = {}
    events_valid = True
    subjects_valid = True
    facts_valid = True
    for scenario_name, scenario in evidence["scenarios"].items():
        provenance = _subject_provenance(foundry, scenario, canonical, evidence)
        subjects_valid = subjects_valid and provenance["complete"]
        records: list[dict[str, Any]] = []
        for transaction_hash in scenario["transactions"]:
            receipt(transaction_hash)
        for event in scenario["events"]:
            transaction_hash = event["transaction_hash"].lower()
            transaction_receipt = receipt(transaction_hash)
            log = _receipt_log(transaction_receipt, event["log_index"])
            topic0 = foundry.keccak_text(EVENT_SIGNATURES[event["name"]])
            block_number = _uint(transaction_receipt.get("blockNumber"))
            expected_contract = (
                canonical["appealable_verifier"]
                if event["name"] in VERIFIER_EVENTS
                else str(scenario["subject_contract"]).lower()
            )
            expected_id = str(
                scenario["case_id"] if event["name"] in VERIFIER_EVENTS else scenario["bounty_id"]
            ).lower()
            passed = False
            if log is not None:
                try:
                    topics = _topics(log)
                    passed = (
                        str(log.get("address", "")).lower() == str(event["contract"]).lower()
                        and str(event["contract"]).lower() == expected_contract
                        and len(topics) >= 2
                        and topics[0] == topic0
                        and topics[1] == expected_id
                        and block_number == event["block_number"]
                    )
                except RehearsalAuditError:
                    passed = False
            events_valid = events_valid and passed
            if log is not None:
                records.append(
                    {
                        "event": event,
                        "receipt": transaction_receipt,
                        "log": log,
                        "block_number": block_number,
                    }
                )
            event_checks.append(
                {
                    "scenario": scenario_name,
                    "event": event["name"],
                    "transaction_hash": transaction_hash,
                    "log_index": event["log_index"],
                    "confirmed": passed,
                }
            )
        try:
            facts = _scenario_fact_checks(foundry, scenario_name, scenario, records)
        except (IndexError, KeyError, RehearsalAuditError):
            facts = {"decoded_onchain_facts": False}
        scenario_facts_valid = bool(facts) and all(facts.values())
        facts_valid = facts_valid and scenario_facts_valid
        scenario_checks[scenario_name] = {
            "provenance": provenance,
            "onchain_facts": facts,
            "complete": provenance["complete"] and scenario_facts_valid,
        }

    funding_transactions = [evidence["faucet_funding"]["native_eth_transaction"]]
    funding_transactions.extend(evidence["faucet_funding"]["test_usdc_transactions"])
    funding_receipts_confirmed = all(bool(receipt(item)) for item in funding_transactions)
    expected_consumers = {canonical["verifier_sortition"], canonical["solver_sortition"]}
    checks = {
        "deployment_rpc_verification": deployment_verification.get("rpc_confirmed") is True,
        "exact_compiled_runtime_bytecode": all(runtime_source_checks.values()),
        "exact_clean_source_revision": local_commit == str(evidence["source_commit"]).lower()
        and not worktree_status,
        "all_required_events_confirmed": events_valid,
        "canonical_subject_provenance": subjects_valid,
        "decoded_onchain_scenario_facts": facts_valid,
        "faucet_receipts_confirmed": funding_receipts_confirmed,
        "minimum_eight_eligible_verifiers": len(set(eligible_verifiers)) >= 8,
        "minimum_three_eligible_solvers": len(set(eligible_solvers)) >= 3,
        "native_subscription_reserve": subscription["native_balance"] >= minimum_subscription,
        "keeper_gas_sponsorship_reserve": keeper_balance >= minimum_gas,
        "exact_two_consumers": set(subscription["consumers"]) == expected_consumers,
        "observation_block_not_ahead_of_rpc": observation_block >= declared_observation["block_number"],
    }
    blockers = [f"{endpoint_label} RPC rehearsal failed {name}" for name, passed in checks.items() if not passed]
    return {
        "endpoint_label": endpoint_label,
        "checks": checks,
        "pass_complete": not blockers,
        "blockers": blockers,
        "event_checks": event_checks,
        "scenario_checks": scenario_checks,
        "eligible_verifier_wallets": eligible_verifiers,
        "eligible_solver_wallets": eligible_solvers,
        "subscription": subscription,
        "keeper_native_balance_wei": keeper_balance,
        "observation_block_number": observation_block,
        "source_commit": local_commit,
        "runtime_source_checks": runtime_source_checks,
        "deployment_verification": deployment_verification,
    }


def audit_live_pair(
    primary: Any,
    secondary: Any,
    evidence: Mapping[str, Any],
    deployment: Mapping[str, Any],
) -> dict[str, Any]:
    passes = [
        audit_live_pass(primary, evidence, deployment, "primary"),
        audit_live_pass(secondary, evidence, deployment, "secondary"),
    ]
    agreement = (
        passes[0]["deployment_verification"]["canonical_component_addresses"]
        == passes[1]["deployment_verification"]["canonical_component_addresses"]
        and passes[0]["subscription"]["owner"] == passes[1]["subscription"]["owner"]
        and set(passes[0]["subscription"]["consumers"]) == set(passes[1]["subscription"]["consumers"])
    )
    blockers = [blocker for item in passes for blocker in item["blockers"]]
    if not agreement:
        blockers.append("independent RPC observations disagree on canonical deployment or subscription authority")
    return {
        "live_complete": all(item["pass_complete"] for item in passes) and agreement,
        "independent_rpc_agreement": agreement,
        "rpc_passes": passes,
        "blockers": blockers,
    }


def audit_rehearsal(
    evidence: Mapping[str, Any],
    live: Mapping[str, Any] | None,
) -> dict[str, Any]:
    structure = audit_structure(evidence)
    rpc_passes = live.get("rpc_passes") if isinstance(live, Mapping) else None
    live_complete = (
        isinstance(live, Mapping)
        and live.get("live_complete") is True
        and live.get("independent_rpc_agreement") is True
        and isinstance(rpc_passes, list)
        and len(rpc_passes) == 2
        and all(isinstance(item, Mapping) and item.get("pass_complete") is True for item in rpc_passes)
    )
    blockers = list(structure["blockers"])
    if live is None:
        blockers.append("live RPC verification is missing")
    else:
        blockers.extend(live.get("blockers", []))
        if not live_complete and not live.get("blockers"):
            blockers.append("live evidence does not contain two complete agreeing RPC passes")
    complete = structure["structure_complete"] and live_complete
    return {
        "schema": AUDIT_SCHEMA,
        "network": evidence.get("network"),
        "chain_id": evidence.get("chain_id"),
        "evidence_content_sha256": evidence.get("content_sha256"),
        "structure": structure,
        "live": live,
        "complete": complete,
        "blockers": blockers,
        "evidence_boundary": (
            "Completion requires live RPC receipt/log, deployment wiring, pool, subscription, and reserve checks. "
            "A JSON claim or transaction hash alone is never rehearsal, settlement, or payment proof."
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--seal-output", type=Path)
    parser.add_argument("--deployment", type=Path)
    parser.add_argument("--rpc-url")
    parser.add_argument("--secondary-rpc-url")
    parser.add_argument("--forge", default="forge")
    parser.add_argument("--cast", default="cast")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--require-complete", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        evidence = read_object(args.evidence)
        if args.seal_output is not None:
            if args.seal_output.resolve() == args.evidence.resolve():
                raise RehearsalAuditError("--seal-output must not overwrite --evidence")
            evidence["content_sha256"] = content_sha256(evidence)
            write_object(args.seal_output, evidence)
        live = None
        if args.deployment is not None or args.rpc_url is not None or args.secondary_rpc_url is not None:
            if args.deployment is None or not args.rpc_url or not args.secondary_rpc_url:
                raise RehearsalAuditError(
                    "--deployment, --rpc-url, and --secondary-rpc-url must be supplied together"
                )
            if args.rpc_url == args.secondary_rpc_url:
                raise RehearsalAuditError("primary and secondary RPC URLs must be distinct")
            if audit_structure(evidence)["structure_complete"]:
                repo = Path(__file__).resolve().parents[1]
                primary = DEPLOY.Foundry(repo, args.rpc_url, args.forge, args.cast)
                secondary = DEPLOY.Foundry(repo, args.secondary_rpc_url, args.forge, args.cast)
                live = audit_live_pair(primary, secondary, evidence, read_object(args.deployment))
            else:
                live = {
                    "live_complete": False,
                    "blockers": ["live RPC audit skipped because rehearsal evidence structure is invalid"],
                }
        result = audit_rehearsal(evidence, live)
        write_object(args.output, result)
        print(f"standing_meta_v4_rehearsal_audit={args.output} complete={str(result['complete']).lower()}")
        return 2 if args.require_complete and not result["complete"] else 0
    except (RehearsalAuditError, DEPLOY.DeploymentError, OSError, json.JSONDecodeError, KeyError, ValueError) as error:
        raise SystemExit(f"standing-meta-v4 rehearsal audit failed: {error}") from error


if __name__ == "__main__":
    raise SystemExit(main())
