#!/usr/bin/env python3
"""Build dual-RPC, fail-closed canonical GMV snapshots for reviewed meta-campaigns."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.request import Request, urlopen

from _shared.evm import address_bytes, keccak_bytes, keccak256
from _shared.rpc import BASE_CHAIN_ID, redact_rpc_endpoint, rpc


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POOL = ROOT / "ops/open-competition-v2-gmv-candidate-pool-v1.json"
DEFAULT_OUTPUT = ROOT / "site/generated/gmv-snapshots"
SCHEMA = "agent-bounties/canonical-gmv-snapshot-v1"
SNAPSHOT_DOMAIN = bytes.fromhex(
    "52abd265a2d2f97ff5791f02f8940cf87dd129db37373e61c16bf1b123b9ec9d"
)
POLICY_DOMAIN = bytes.fromhex(
    "2ca0d81d158e559a56f86e206b4e7131939657aa1d3eb7c74efc1b88f92fd833"
)
EXCLUSIONS_DOMAIN = bytes.fromhex(
    "c6b6b1da9249908bdb0412604fe8ddc48caa98251c069921a6de76b150af5d43"
)
EPOCH_DOMAIN = b"agent-bounties/canonical-gmv-epoch-v1\0"


class SnapshotError(RuntimeError):
    pass


@dataclass(frozen=True)
class Protocol:
    name: str
    tag: int
    events_path: str
    factory: str
    deployment_block: int
    creation_kind: str
    creation_signature: str
    settlement_kind: str
    settlement_signature: str
    funding_signature: str


PROTOCOLS = (
    Protocol(
        "autonomous",
        0,
        "/v1/base/autonomous-bounties/events?network=base-mainnet",
        "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        48_496_662,
        "canonical_bounty_created",
        "CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)",
        "bounty_settled",
        "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
        "FundingAdded(bytes32,address,uint256,uint256,uint256)",
    ),
    Protocol(
        "open_competition_v1",
        1,
        "/v1/base/open-competition-v1/events?network=base-mainnet",
        "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5",
        49_663_931,
        "canonical_competition_created",
        "CanonicalCompetitionCreated(bytes32,address,address,bytes32,bytes32,bytes32)",
        "bounty_settled",
        "BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)",
        "FundingAdded(bytes32,address,uint256,uint256,uint256)",
    ),
    Protocol(
        "open_competition_v2",
        2,
        "/v1/base/open-competition-v2-beta3/events?network=base-mainnet",
        "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        50_213_074,
        "canonical_competition_created",
        "CanonicalCompetitionCreatedV2(bytes32,address,address,bytes32,bytes32)",
        "competition_settled",
        "CompetitionSettledV2(bytes32,uint256,address,uint256,address,uint256,bytes32,bytes32,int256,bytes32)",
        "FundingAddedV2(bytes32,address,uint256,uint256,uint256)",
    ),
)


def fetch_json(url: str) -> Any:
    request = Request(url, headers={"accept": "application/json", "user-agent": "agent-bounties-gmv-snapshot/1"})
    with urlopen(request, timeout=30) as response:
        return json.load(response)


def parse_time(value: object, field: str) -> datetime:
    if not isinstance(value, str):
        raise SnapshotError(f"{field} must be an RFC3339 timestamp")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise SnapshotError(f"{field} must be an RFC3339 timestamp") from error
    if parsed.tzinfo is None:
        raise SnapshotError(f"{field} must include a timezone")
    return parsed.astimezone(timezone.utc)


def parse_quantity(value: object, field: str) -> int:
    if not isinstance(value, str) or not value.startswith("0x"):
        raise SnapshotError(f"{field} is not an RPC quantity")
    try:
        return int(value, 16)
    except ValueError as error:
        raise SnapshotError(f"{field} is not an RPC quantity") from error


def bytes32(value: object, field: str) -> bytes:
    if not isinstance(value, str) or not value.startswith("0x") or len(value) != 66:
        raise SnapshotError(f"{field} must be bytes32 hex")
    try:
        decoded = bytes.fromhex(value[2:])
    except ValueError as error:
        raise SnapshotError(f"{field} must be bytes32 hex") from error
    if decoded == bytes(32):
        raise SnapshotError(f"{field} must be nonzero")
    return decoded


def normalize_address(value: object, field: str) -> str:
    if not isinstance(value, str):
        raise SnapshotError(f"{field} must be an address")
    try:
        address_bytes(value)
    except ValueError as error:
        raise SnapshotError(f"{field} must be an address") from error
    return value.lower()


def topic(signature: str) -> str:
    return keccak256(signature.encode("ascii"))


def raw_log_identity(log: dict[str, Any]) -> dict[str, Any]:
    return {
        "address": normalize_address(log.get("address"), "log.address"),
        "block_hash": "0x" + bytes32(log.get("blockHash"), "log.blockHash").hex(),
        "block_number": parse_quantity(log.get("blockNumber"), "log.blockNumber"),
        "data": str(log.get("data") or "").lower(),
        "log_index": parse_quantity(log.get("logIndex"), "log.logIndex"),
        "topics": [str(value).lower() for value in log.get("topics") or []],
        "transaction_hash": "0x" + bytes32(log.get("transactionHash"), "log.transactionHash").hex(),
    }


class RpcPair:
    def __init__(self, primary: str, shadow: str, span: int) -> None:
        if primary.strip() == shadow.strip():
            raise SnapshotError("primary and shadow RPC endpoints must be independent")
        if span < 1 or span > 100_000:
            raise SnapshotError("RPC block span must be between 1 and 100000")
        self.primary = primary
        self.shadow = shadow
        self.span = span
        self.request_id = 40_000
        self.blocks: dict[tuple[str, int], dict[str, Any]] = {}
        self.receipts: dict[tuple[str, str], dict[str, Any]] = {}

    def call(self, endpoint: str, method: str, params: list[Any]) -> Any:
        self.request_id += 1
        return rpc(endpoint, method, params, self.request_id, attempts=3, timeout=30)

    def validate_chains(self) -> None:
        for label, endpoint in (("primary", self.primary), ("shadow", self.shadow)):
            chain = parse_quantity(self.call(endpoint, "eth_chainId", []), f"{label} chain id")
            if chain != BASE_CHAIN_ID:
                raise SnapshotError(f"{label} RPC is not Base mainnet")

    def block(self, endpoint: str, number: int) -> dict[str, Any]:
        key = (endpoint, number)
        if key not in self.blocks:
            result = self.call(endpoint, "eth_getBlockByNumber", [hex(number), False])
            if not isinstance(result, dict):
                raise SnapshotError(f"block {number} is unavailable")
            self.blocks[key] = result
        return self.blocks[key]

    def exact_block(self, number: int) -> dict[str, Any]:
        primary = self.block(self.primary, number)
        shadow = self.block(self.shadow, number)
        fields = ("hash", "number", "timestamp")
        if any(str(primary.get(field)).lower() != str(shadow.get(field)).lower() for field in fields):
            raise SnapshotError(f"RPCs disagree on block {number}")
        return primary

    def safe_head(self) -> int:
        heads = []
        for endpoint in (self.primary, self.shadow):
            block = self.call(endpoint, "eth_getBlockByNumber", ["safe", False])
            if not isinstance(block, dict):
                raise SnapshotError("RPC did not return a safe block")
            heads.append(parse_quantity(block.get("number"), "safe block number"))
        head = min(heads)
        self.exact_block(head)
        return head

    def last_block_before(self, timestamp: int, safe_head: int) -> int:
        low, high = 1, safe_head
        while low <= high:
            middle = (low + high) // 2
            observed = parse_quantity(self.block(self.primary, middle).get("timestamp"), "block timestamp")
            if observed < timestamp:
                low = middle + 1
            else:
                high = middle - 1
        if high < 1:
            raise SnapshotError("no block exists before the epoch boundary")
        self.exact_block(high)
        return high

    def first_block_at_or_after(self, timestamp: int, safe_head: int) -> int:
        previous = self.last_block_before(timestamp, safe_head)
        result = previous + 1
        self.exact_block(result)
        return result

    def receipt_log(self, tx_hash: str, log_index: int) -> dict[str, Any]:
        identities = []
        for endpoint in (self.primary, self.shadow):
            key = (endpoint, tx_hash)
            if key not in self.receipts:
                receipt = self.call(endpoint, "eth_getTransactionReceipt", [tx_hash])
                if not isinstance(receipt, dict):
                    raise SnapshotError(f"receipt {tx_hash} is unavailable")
                self.receipts[key] = receipt
            matches = [
                log
                for log in self.receipts[key].get("logs") or []
                if parse_quantity(log.get("logIndex"), "receipt log index") == log_index
            ]
            if len(matches) != 1:
                raise SnapshotError(f"receipt {tx_hash} has no unique log {log_index}")
            identities.append(raw_log_identity(matches[0]))
        if identities[0] != identities[1]:
            raise SnapshotError(f"RPCs disagree on {tx_hash}:{log_index}")
        return identities[0]

    def logs(self, from_block: int, to_block: int, event_topic: str, address: str | None = None) -> list[dict[str, Any]]:
        projections: list[list[dict[str, Any]]] = [[], []]
        endpoints = (self.primary, self.shadow)
        for start in range(from_block, to_block + 1, self.span):
            end = min(to_block, start + self.span - 1)
            query: dict[str, Any] = {
                "fromBlock": hex(start),
                "toBlock": hex(end),
                "topics": [event_topic],
            }
            if address is not None:
                query["address"] = address
            for index, endpoint in enumerate(endpoints):
                result = self.call(endpoint, "eth_getLogs", [query])
                if not isinstance(result, list):
                    raise SnapshotError("eth_getLogs did not return a list")
                projections[index].extend(raw_log_identity(log) for log in result)
        for projection in projections:
            projection.sort(key=lambda value: (value["block_number"], value["transaction_hash"], value["log_index"]))
        if projections[0] != projections[1]:
            raise SnapshotError("primary and shadow RPC event sets disagree")
        return projections[0]


def api_events(api_base: str) -> dict[str, list[dict[str, Any]]]:
    result: dict[str, list[dict[str, Any]]] = {}
    for protocol in PROTOCOLS:
        value = fetch_json(api_base.rstrip("/") + protocol.events_path)
        events = value.get("events") if isinstance(value, dict) else value
        if not isinstance(events, list):
            raise SnapshotError(f"{protocol.name} API event response is malformed")
        result[protocol.name] = events
    return result


def api_key(event: dict[str, Any]) -> tuple[int, str, int, str]:
    return (
        int(event["block_number"]),
        str(event["tx_hash"]).lower(),
        int(event["log_index"]),
        str(event["contract_address"]).lower(),
    )


def verify_api_event(pair: RpcPair, event: dict[str, Any], signature: str) -> dict[str, Any]:
    raw = pair.receipt_log(str(event["tx_hash"]).lower(), int(event["log_index"]))
    if raw["block_number"] != int(event["block_number"]):
        raise SnapshotError("API and RPC block numbers disagree")
    if raw["address"] != str(event["contract_address"]).lower():
        raise SnapshotError("API and RPC contract addresses disagree")
    if not raw["topics"] or raw["topics"][0] != topic(signature):
        raise SnapshotError(f"API event does not match {signature}")
    return raw


def word(data: str, index: int) -> bytes:
    raw = bytes.fromhex(data.removeprefix("0x"))
    start = index * 32
    value = raw[start : start + 32]
    if len(value) != 32:
        raise SnapshotError("event data is shorter than its ABI")
    return value


def topic_address(value: str) -> str:
    raw = bytes32(value, "address topic")
    if raw[:12] != bytes(12):
        raise SnapshotError("indexed address is not ABI encoded")
    return "0x" + raw[12:].hex()


def raw_snapshot_record(
    protocol: Protocol,
    settlement: dict[str, Any],
    settlement_log: dict[str, Any],
    creation_log: dict[str, Any],
    funding_logs: list[dict[str, Any]],
    settled_at: int,
) -> dict[str, Any]:
    if creation_log["topics"][1] != str(settlement["bounty_id"]).lower():
        raise SnapshotError("creation bounty id does not match settlement")
    bounty_contract = topic_address(creation_log["topics"][2])
    creator = topic_address(creation_log["topics"][3])
    if bounty_contract != settlement_log["address"]:
        raise SnapshotError("creation contract does not match settlement")
    if settlement_log["topics"][1] != str(settlement["bounty_id"]).lower():
        raise SnapshotError("settlement bounty id does not match API")
    solver = topic_address(settlement_log["topics"][3])
    if protocol.name == "open_competition_v2":
        gmv = int.from_bytes(word(settlement_log["data"], 0), "big") + int.from_bytes(
            word(settlement_log["data"], 2), "big"
        )
    else:
        gmv = (
            int.from_bytes(word(settlement_log["data"], 0), "big")
            + int.from_bytes(word(settlement_log["data"], 2), "big")
            + int.from_bytes(word(settlement_log["data"], 3), "big")
        )
    funding: dict[str, int] = {}
    for log in funding_logs:
        if log["topics"][1] != str(settlement["bounty_id"]).lower() or log["address"] != bounty_contract:
            raise SnapshotError("funding event scope does not match settlement")
        contributor = topic_address(log["topics"][2])
        amount = int.from_bytes(word(log["data"], 0), "big")
        if amount <= 0:
            raise SnapshotError("funding amount must be positive")
        funding[contributor] = funding.get(contributor, 0) + amount
    if not funding:
        raise SnapshotError("settled bounty has no canonical funding events")
    return {
        "protocol": protocol.name,
        "bounty_contract": bounty_contract,
        "bounty_id": str(settlement["bounty_id"]).lower(),
        "creator": creator,
        "solver": solver,
        "settled_at": settled_at,
        "block_number": settlement_log["block_number"],
        "transaction_hash": settlement_log["transaction_hash"],
        "log_index": settlement_log["log_index"],
        "gmv_base_units": gmv,
        "funding": [
            {"contributor": contributor, "amount_base_units": amount}
            for contributor, amount in sorted(funding.items())
        ],
    }


def uint(value: int, length: int) -> bytes:
    if value < 0 or value >= 1 << (length * 8):
        raise SnapshotError("integer is out of range")
    return value.to_bytes(length, "big")


def exclusions_hash(wallets: list[str], contracts: list[str]) -> bytes:
    data = bytearray(EXCLUSIONS_DOMAIN)
    data.extend(uint(len(wallets), 4))
    for value in wallets:
        data.extend(address_bytes(value))
    data.extend(uint(len(contracts), 4))
    for value in contracts:
        data.extend(address_bytes(value))
    return keccak_bytes(bytes(data))


def snapshot_hash(campaign: dict[str, Any], settlements: list[dict[str, Any]]) -> bytes:
    data = bytearray(SNAPSHOT_DOMAIN)
    data.extend(uint(BASE_CHAIN_ID, 8))
    data.extend(bytes32(campaign["epoch_id"], "epoch id"))
    for field in ("starts_at", "ends_at", "start_block", "end_safe_block"):
        data.extend(uint(int(campaign[field]), 8))
    data.extend(bytes32(campaign["end_block_hash"], "end block hash"))
    data.extend(uint(int(campaign["minimum_score_base_units"]), 16))
    data.extend(exclusions_hash(campaign["excluded_wallets"], campaign["excluded_bounty_contracts"]))
    data.extend(uint(len(settlements), 4))
    tags = {protocol.name: protocol.tag for protocol in PROTOCOLS}
    for settlement in settlements:
        data.append(tags[settlement["protocol"]])
        data.extend(address_bytes(settlement["bounty_contract"]))
        data.extend(bytes32(settlement["bounty_id"], "bounty id"))
        data.extend(address_bytes(settlement["creator"]))
        data.extend(address_bytes(settlement["solver"]))
        data.extend(uint(settlement["settled_at"], 8))
        data.extend(uint(settlement["block_number"], 8))
        data.extend(bytes32(settlement["transaction_hash"], "transaction hash"))
        data.extend(uint(settlement["log_index"], 4))
        data.extend(uint(settlement["gmv_base_units"], 16))
        data.extend(uint(len(settlement["funding"]), 4))
        for funding in settlement["funding"]:
            data.extend(address_bytes(funding["contributor"]))
            data.extend(uint(funding["amount_base_units"], 16))
    return keccak_bytes(bytes(data))


def verification_policy_hash(campaign: dict[str, Any], source_hash: str, snapshot: bytes) -> bytes:
    data = bytearray(POLICY_DOMAIN)
    data.extend(uint(BASE_CHAIN_ID, 8))
    data.extend(bytes32(campaign["epoch_id"], "epoch id"))
    for field in ("starts_at", "ends_at", "start_block", "end_safe_block"):
        data.extend(uint(int(campaign[field]), 8))
    data.extend(bytes32(campaign["end_block_hash"], "end block hash"))
    data.extend(uint(int(campaign["minimum_score_base_units"]), 16))
    data.extend(bytes32(source_hash, "source hash"))
    data.extend(snapshot)
    data.extend(exclusions_hash(campaign["excluded_wallets"], campaign["excluded_bounty_contracts"]))
    return keccak_bytes(bytes(data))


def eligible_scores(settlements: list[dict[str, Any]], excluded_wallets: set[str], excluded_contracts: set[str]) -> dict[str, int]:
    scores: dict[str, int] = {}
    for settlement in settlements:
        if settlement["bounty_contract"] in excluded_contracts or settlement["creator"] == settlement["solver"]:
            continue
        total = sum(item["amount_base_units"] for item in settlement["funding"])
        for item in settlement["funding"]:
            entrant = item["contributor"]
            if entrant in excluded_wallets or entrant == settlement["solver"]:
                continue
            attributed = settlement["gmv_base_units"] * item["amount_base_units"] // total
            scores[entrant] = scores.get(entrant, 0) + attributed
    return {wallet: score for wallet, score in scores.items() if score > 0}


def reconcile_event_sets(pair: RpcPair, events: dict[str, list[dict[str, Any]]], maximum_block: int) -> None:
    known_contracts: set[str] = set()
    for protocol in PROTOCOLS:
        creations = [event for event in events[protocol.name] if event.get("kind") == protocol.creation_kind]
        raw_creations = pair.logs(protocol.deployment_block, maximum_block, topic(protocol.creation_signature), protocol.factory)
        api_creation_keys = {api_key(event) for event in creations if int(event["block_number"]) <= maximum_block}
        raw_creation_keys = {
            (log["block_number"], log["transaction_hash"], log["log_index"], log["address"])
            for log in raw_creations
        }
        if api_creation_keys != raw_creation_keys:
            raise SnapshotError(f"{protocol.name} API and RPC creation event sets disagree")
        for event in creations:
            contract_field = "competition" if protocol.name == "open_competition_v2" else "bounty_contract"
            known_contracts.add(normalize_address(event["data"][contract_field], "created contract"))
    minimum_block = min(protocol.deployment_block for protocol in PROTOCOLS)
    for signature in {protocol.settlement_signature for protocol in PROTOCOLS}:
        raw = pair.logs(minimum_block, maximum_block, topic(signature))
        raw_keys = {
            (log["block_number"], log["transaction_hash"], log["log_index"], log["address"])
            for log in raw
            if log["address"] in known_contracts
        }
        api_keys = {
            api_key(event)
            for protocol in PROTOCOLS
            if protocol.settlement_signature == signature
            for event in events[protocol.name]
            if event.get("kind") == protocol.settlement_kind and int(event["block_number"]) <= maximum_block
        }
        if raw_keys != api_keys:
            raise SnapshotError("canonical API and dual-RPC settlement event sets disagree")


def build_snapshots(
    pool: dict[str, Any],
    events: dict[str, list[dict[str, Any]]],
    pair: RpcPair,
    candidate_ids: set[str] | None,
) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    selected = [
        candidate
        for candidate in pool["candidates"]
        if candidate_ids is None or candidate["candidate_id"] in candidate_ids
    ]
    if candidate_ids is not None and {item["candidate_id"] for item in selected} != candidate_ids:
        raise SnapshotError("one or more requested candidate ids are unknown")
    safe_head = pair.safe_head()
    boundaries: dict[int, tuple[int, dict[str, Any]]] = {}
    for candidate in selected:
        end = int(parse_time(candidate["epoch"]["ends_at"], "epoch.ends_at").timestamp())
        end_block = pair.last_block_before(end, safe_head)
        boundaries[end] = (end_block, pair.exact_block(end_block))
    maximum_block = max(value[0] for value in boundaries.values())
    reconcile_event_sets(pair, events, maximum_block)
    documents: dict[str, dict[str, Any]] = {}
    snapshot_fields: dict[str, dict[str, Any]] = {}
    policy = pool.get("eligibility_policy")
    if not isinstance(policy, dict):
        raise SnapshotError("candidate pool eligibility_policy is required")
    excluded_wallets = sorted(normalize_address(value, "excluded wallet") for value in policy.get("excluded_wallets") or [])
    excluded_contracts = sorted(normalize_address(value, "excluded contract") for value in policy.get("excluded_bounty_contracts") or [])
    if not excluded_wallets or not excluded_contracts or len(set(excluded_wallets)) != len(excluded_wallets) or len(set(excluded_contracts)) != len(excluded_contracts):
        raise SnapshotError("eligibility exclusions must be nonempty, unique, and sorted")
    source_hash = pool["profile_release"]["source_hash"]
    reconciled_at = datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    for candidate in selected:
        candidate_id = candidate["candidate_id"]
        starts_at = int(parse_time(candidate["epoch"]["starts_at"], "epoch.starts_at").timestamp())
        ends_at = int(parse_time(candidate["epoch"]["ends_at"], "epoch.ends_at").timestamp())
        start_block = pair.first_block_at_or_after(starts_at, safe_head)
        end_block, end_header = boundaries[ends_at]
        campaign = {
            "epoch_id": "0x" + keccak_bytes(EPOCH_DOMAIN + candidate_id.encode("utf-8")).hex(),
            "starts_at": starts_at,
            "ends_at": ends_at,
            "start_block": start_block,
            "end_safe_block": end_block,
            "end_block_hash": str(end_header["hash"]).lower(),
            "minimum_score_base_units": int(candidate["epoch"]["minimum_score_base_units"]),
            "excluded_wallets": excluded_wallets,
            "excluded_bounty_contracts": excluded_contracts,
        }
        records: list[dict[str, Any]] = []
        evidence: list[dict[str, Any]] = []
        for protocol in PROTOCOLS:
            protocol_events = events[protocol.name]
            creations = {event["bounty_id"]: event for event in protocol_events if event.get("kind") == protocol.creation_kind}
            funding_by_bounty: dict[str, list[dict[str, Any]]] = {}
            for event in protocol_events:
                if event.get("kind") == "funding_added":
                    funding_by_bounty.setdefault(event["bounty_id"], []).append(event)
            settlements = [
                event
                for event in protocol_events
                if event.get("kind") == protocol.settlement_kind
                and starts_at <= int(parse_time(event["occurred_at"], "event.occurred_at").timestamp()) < ends_at
            ]
            for settlement in settlements:
                creation = creations.get(settlement["bounty_id"])
                funding = funding_by_bounty.get(settlement["bounty_id"], [])
                if creation is None or not funding:
                    raise SnapshotError("canonical settlement lacks creation or funding evidence")
                settlement_raw = verify_api_event(pair, settlement, protocol.settlement_signature)
                creation_raw = verify_api_event(pair, creation, protocol.creation_signature)
                funding_raw = [verify_api_event(pair, item, protocol.funding_signature) for item in funding]
                block = pair.exact_block(int(settlement["block_number"]))
                settled_at = parse_quantity(block["timestamp"], "settlement block timestamp")
                if settled_at != int(parse_time(settlement["occurred_at"], "event.occurred_at").timestamp()):
                    raise SnapshotError("API settlement timestamp disagrees with canonical block")
                records.append(raw_snapshot_record(protocol, settlement, settlement_raw, creation_raw, funding_raw, settled_at))
                evidence.append({
                    "protocol": protocol.name,
                    "transaction_hash": settlement_raw["transaction_hash"],
                    "log_index": settlement_raw["log_index"],
                    "block_hash": settlement_raw["block_hash"],
                })
        records.sort(key=lambda value: (value["block_number"], value["transaction_hash"], value["log_index"], value["bounty_contract"], value["bounty_id"]))
        if not records:
            raise SnapshotError(f"{candidate_id} contains no canonical settlement")
        scores = eligible_scores(records, set(excluded_wallets), set(excluded_contracts))
        if not scores:
            raise SnapshotError(f"{candidate_id} contains no eligible external-funder GMV")
        digest = snapshot_hash(campaign, records)
        policy_digest = verification_policy_hash(campaign, source_hash, digest)
        document = {
            "schema": SCHEMA,
            "candidate_id": candidate_id,
            "network": "base-mainnet",
            "chain_id": BASE_CHAIN_ID,
            "campaign": campaign,
            "settlements": records,
            "snapshot_hash": "0x" + digest.hex(),
            "verification_policy_hash": "0x" + policy_digest.hex(),
            "eligible_wallet_count": len(scores),
            "eligible_gmv_base_units": str(sum(scores.values())),
            "canonical_evidence": evidence,
            "reconciliation": {
                "status": "primary_shadow_agree",
                "primary_rpc": redact_rpc_endpoint(pair.primary),
                "shadow_rpc": redact_rpc_endpoint(pair.shadow),
                "reconciled_at": reconciled_at,
                "evidence_boundary": "API event projections were reproduced from exact logs and block identities returned by two independent Base RPCs. Only canonical settlement events contribute GMV.",
            },
        }
        documents[candidate_id] = document
        snapshot_fields[candidate_id] = {
            "status": "ready",
            "safe_block": end_block,
            "end_block_hash": campaign["end_block_hash"],
            "snapshot_hash": document["snapshot_hash"],
            "verification_policy_hash": document["verification_policy_hash"],
            "primary_projection_hash": document["snapshot_hash"],
            "shadow_projection_hash": document["snapshot_hash"],
            "snapshot_url": f"https://agentbounties.app/generated/gmv-snapshots/{candidate_id}.json",
            "reconciled_at": reconciled_at,
        }
    return documents, snapshot_fields


def write_outputs(output: Path, documents: dict[str, dict[str, Any]], snapshots: dict[str, dict[str, Any]], pool_output: Path | None, pool: dict[str, Any]) -> None:
    output.mkdir(parents=True, exist_ok=True)
    for candidate_id, document in documents.items():
        (output / f"{candidate_id}.json").write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    if pool_output is not None:
        updated = json.loads(json.dumps(pool))
        for candidate in updated["candidates"]:
            if candidate["candidate_id"] in snapshots:
                candidate["snapshot"] = snapshots[candidate["candidate_id"]]
        pool_output.write_text(json.dumps(updated, indent=2) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate-pool", type=Path, default=DEFAULT_POOL)
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--pool-output", type=Path)
    parser.add_argument("--api-base-url", default="https://api.agentbounties.app")
    parser.add_argument("--primary-rpc", required=True)
    parser.add_argument("--shadow-rpc", required=True)
    parser.add_argument("--rpc-block-span", type=int, default=50_000)
    parser.add_argument("--candidate-id", action="append", dest="candidate_ids")
    args = parser.parse_args(argv)
    try:
        pool = json.loads(args.candidate_pool.read_text(encoding="utf-8-sig"))
        pair = RpcPair(args.primary_rpc, args.shadow_rpc, args.rpc_block_span)
        pair.validate_chains()
        events = api_events(args.api_base_url)
        documents, snapshots = build_snapshots(
            pool, events, pair, set(args.candidate_ids) if args.candidate_ids else None
        )
        write_outputs(args.output_dir, documents, snapshots, args.pool_output, pool)
    except (OSError, ValueError, json.JSONDecodeError, SnapshotError, RuntimeError) as error:
        print(f"GMV snapshot build blocked: {error}", file=sys.stderr)
        return 2
    summary = {
        "status": "ready",
        "candidate_count": len(documents),
        "snapshot_set_hash": "0x" + hashlib.sha256(
            json.dumps(snapshots, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest(),
    }
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
