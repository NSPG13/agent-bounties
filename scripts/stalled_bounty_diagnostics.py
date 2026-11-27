#!/usr/bin/env python3
"""Diagnose stalled claimed and submitted agent bounties before deadlines strand work or escrow.

Strictly non-authoritative and read-only: derives deterministic next actions and deadlines
solely from canonical on-chain lifecycle events, immutable contract parameters, and live verifier
readiness. Never infers state from GitHub, unverified transaction hashes, or AI opinions.
"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import sys
import time
from typing import Any, Mapping, Sequence


SCHEMA_VERSION = "agent-bounties/stalled-bounty-diagnostics-v1"
DISCOVERY_SCHEMA = "agent-bounties/stalled-bounty-discovery-v1"
FIXTURES_SCHEMA = "agent-bounties/stalled-bounty-fixtures-v1"

# Canonical Lifecycle States (Matching AgentBounty.sol enum BountyStatus)
STATUS_OPEN = "open"
STATUS_CLAIMABLE = "claimable"
STATUS_CLAIMED = "claimed"
STATUS_SUBMITTED = "submitted"
STATUS_SETTLED = "settled"
STATUS_CANCELLED = "cancelled"

CANONICAL_STATUSES = frozenset(
    {STATUS_OPEN, STATUS_CLAIMABLE, STATUS_CLAIMED, STATUS_SUBMITTED, STATUS_SETTLED, STATUS_CANCELLED}
)

# Diagnostic Classifications
CLASS_HEALTHY_CLAIMED = "healthy_claimed"
CLASS_CLAIM_EXPIRING = "claim_expiring"
CLASS_SUBMITTED = "submitted"
CLASS_VERIFICATION_EXPIRING = "verification_expiring"
CLASS_VERIFIER_UNAVAILABLE = "verifier_unavailable"
CLASS_SETTLED = "settled"
CLASS_TERMINAL = "terminal"

DIAGNOSTIC_CLASSIFICATIONS = frozenset(
    {
        CLASS_HEALTHY_CLAIMED,
        CLASS_CLAIM_EXPIRING,
        CLASS_SUBMITTED,
        CLASS_VERIFICATION_EXPIRING,
        CLASS_VERIFIER_UNAVAILABLE,
        CLASS_SETTLED,
        CLASS_TERMINAL,
    }
)

# Canonical Lifecycle Events
EVENT_FUNDING_ADDED = "FundingAdded"
EVENT_BOUNTY_CLAIMABLE = "BountyBecameClaimable"
EVENT_BOUNTY_CLAIMED = "BountyClaimed"
EVENT_SUBMISSION_ADDED = "SubmissionAdded"
EVENT_SUBMISSION_REJECTED = "SubmissionRejected"
EVENT_BOUNTY_SETTLED = "BountySettled"
EVENT_CLAIM_EXPIRED = "ClaimExpired"
EVENT_SUBMISSION_EXPIRED = "SubmissionExpired"
EVENT_BOUNTY_CANCELLED = "BountyCancelled"
EVENT_REFUND_WITHDRAWN = "RefundWithdrawn"

CANONICAL_EVENTS = frozenset(
    {
        EVENT_FUNDING_ADDED,
        EVENT_BOUNTY_CLAIMABLE,
        EVENT_BOUNTY_CLAIMED,
        EVENT_SUBMISSION_ADDED,
        EVENT_SUBMISSION_REJECTED,
        EVENT_BOUNTY_SETTLED,
        EVENT_CLAIM_EXPIRED,
        EVENT_SUBMISSION_EXPIRED,
        EVENT_BOUNTY_CANCELLED,
        EVENT_REFUND_WITHDRAWN,
    }
)

# Deterministic Next Actions
ACTION_SUBMIT_WORK = "submit_work"
ACTION_EXPIRE_CLAIM = "expire_claim"
ACTION_ATTEST_AND_SETTLE = "attest_and_settle"
ACTION_URGENT_ATTEST_AND_SETTLE = "urgent_attest_and_settle"
ACTION_EXPIRE_SUBMISSION = "expire_submission"
ACTION_FAILOVER_VERIFIER = "failover_verifier"
ACTION_RESTORE_VERIFIER_FLEET = "restore_verifier_fleet"
ACTION_RESOLVE_MISSING_TERMS = "resolve_missing_terms"
ACTION_REFRESH_INDEX_STATE = "refresh_index_state"
ACTION_CANCEL_UNFUNDED = "cancel_unfunded"

# Threshold defaults
DEFAULT_CLAIM_WARNING_SECONDS = 86400  # 24 hours
DEFAULT_VERIFICATION_WARNING_SECONDS = 3600  # 1 hour
MAX_ACCEPTABLE_INDEXER_LAG_SECONDS = 300  # 5 minutes
MAX_ACCEPTABLE_INDEXER_LAG_BLOCKS = 64


class CanonicalIntegrityError(ValueError):
    """Raised when non-canonical or untrusted signals attempt to infer state."""


@dataclass(frozen=True)
class CanonicalEvent:
    """A verified on-chain lifecycle event log."""
    event_name: str
    block_number: int
    block_timestamp: int
    tx_hash: str
    payload: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class BountyContractConfig:
    """Immutable contract and terms parameters for an agent bounty."""
    bounty_id: str
    contract_address: str
    title: str = ""
    creator: str = ""
    settlement_token: str = ""
    solver_reward: int = 0
    verifier_reward: int = 0
    claim_bond: int = 0
    funding_deadline: int = 0
    claim_window_seconds: int = 604800
    verification_window_seconds: int = 7200
    terms_hash: str = ""
    policy_hash: str = ""
    verification_mode: str = "signed_quorum"
    verifier_module: str = ""
    verifiers: tuple[str, ...] = field(default_factory=tuple)
    threshold: int = 1

    def to_dict(self) -> dict[str, Any]:
        data = asdict(self)
        data["verifiers"] = list(self.verifiers)
        return data


@dataclass(frozen=True)
class BountyLifecycleState:
    """Live or reconstructed state of a bounty contract."""
    status: str
    round: int = 0
    solver: str | None = None
    claim_expires_at: int | None = None
    verification_expires_at: int | None = None
    submission_hash: str | None = None
    evidence_hash: str | None = None
    active_claim_bond: int = 0
    timeout_bond_pool: int = 0

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class VerifierFleetStatus:
    """Health and reachability of the designated verifier quorum or module."""
    verifiers_available: bool = True
    healthy_count: int = 2
    required_threshold: int = 2
    error: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class IndexerSyncStatus:
    """Cursor and freshness metrics of the underlying blockchain indexer."""
    is_stale: bool = False
    lag_blocks: int = 0
    heartbeat_age_seconds: int = 0

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class BountyItemDiagnosis:
    """Deterministic diagnosis and single recovery action for a single bounty."""
    bounty_id: str
    contract_address: str
    title: str
    current_status: str
    classification: str
    is_stalled: bool
    is_terminal: bool
    round: int
    solver: str | None
    claim_expires_at: int | None
    verification_expires_at: int | None
    next_action: str | None
    deadline: int | None
    deadline_iso: str | None
    seconds_remaining: int | None
    urgency: str
    reason: str
    verifier_status: dict[str, Any] = field(default_factory=dict)
    canonical_evidence: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class DiagnosticsReport:
    """Aggregate diagnostics report across the entire bounty inventory backlog."""
    schema_version: str
    observed_at: str
    reference_timestamp: int
    summary: dict[str, int]
    items: list[BountyItemDiagnosis]
    backlog: list[BountyItemDiagnosis]
    operations_markdown: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "observed_at": self.observed_at,
            "reference_timestamp": self.reference_timestamp,
            "summary": self.summary,
            "items": [item.to_dict() for item in self.items],
            "backlog": [item.to_dict() for item in self.backlog],
            "operations_markdown": self.operations_markdown,
        }


def format_iso_timestamp(timestamp: int | None) -> str | None:
    """Format integer unix timestamp into ISO 8601 UTC string."""
    if timestamp is None or timestamp <= 0:
        return None
    return datetime.fromtimestamp(timestamp, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_canonical_events(raw_events: Sequence[Mapping[str, Any]]) -> list[CanonicalEvent]:
    """Parse and validate canonical on-chain lifecycle events."""
    events: list[CanonicalEvent] = []
    for item in raw_events:
        name = str(item.get("event", item.get("name", ""))).strip()
        block_number = int(item.get("block_number", item.get("block", 0)))
        block_timestamp = int(item.get("block_timestamp", item.get("timestamp", 0)))
        tx_hash = str(item.get("tx_hash", item.get("transaction_hash", ""))).strip().lower()
        payload = dict(item.get("payload", item.get("args", {})))
        events.append(
            CanonicalEvent(
                event_name=name,
                block_number=block_number,
                block_timestamp=block_timestamp,
                tx_hash=tx_hash,
                payload=payload,
            )
        )
    return events


def diagnose_bounty(
    contract: BountyContractConfig,
    lifecycle_state: BountyLifecycleState,
    canonical_events: Sequence[CanonicalEvent] = (),
    verifier_status: VerifierFleetStatus | None = None,
    indexer_status: IndexerSyncStatus | None = None,
    has_valid_terms_document: bool = True,
    reference_timestamp: int | None = None,
    external_github_state: Mapping[str, Any] | None = None,
    ai_opinion: Mapping[str, Any] | None = None,
) -> BountyItemDiagnosis:
    """Diagnose a single bounty contract against canonical lifecycle rules and deadlines.

    Guarantees:
    - Never infers settlement, rejection, or claims from GitHub state or AI opinion.
    - Deterministically classifies healthy, expiring, submitted, verifier outage, and settled states.
    - Yields exactly one deterministic next_action and deadline for non-terminal stalled work.
    """
    now = int(reference_timestamp if reference_timestamp is not None else time.time())
    v_status = verifier_status or VerifierFleetStatus()
    idx_status = indexer_status or IndexerSyncStatus()

    # Invariant Guard: Assert that untrusted signals are not used to fabricate canonical state
    if external_github_state is not None and external_github_state.get("simulate_settlement"):
        raise CanonicalIntegrityError("Cannot infer settlement from external GitHub state")
    if ai_opinion is not None and ai_opinion.get("verdict") == "settled":
        raise CanonicalIntegrityError("Cannot infer settlement from AI opinion")

    # Check for verified terminal canonical events
    event_names = [e.event_name.lower() for e in canonical_events]
    has_settled_event = any(name in {"bountysettled", "bounty_settled"} for name in event_names)
    has_cancelled_event = any(name in {"bountycancelled", "bounty_cancelled"} for name in event_names)

    status = lifecycle_state.status.lower()

    # 1. Settled State Check (Canonical BountySettled event or on-chain status Settled)
    if status == STATUS_SETTLED or has_settled_event:
        return BountyItemDiagnosis(
            bounty_id=contract.bounty_id,
            contract_address=contract.contract_address,
            title=contract.title,
            current_status=STATUS_SETTLED,
            classification=CLASS_SETTLED,
            is_stalled=False,
            is_terminal=True,
            round=lifecycle_state.round,
            solver=lifecycle_state.solver,
            claim_expires_at=lifecycle_state.claim_expires_at,
            verification_expires_at=lifecycle_state.verification_expires_at,
            next_action=None,
            deadline=None,
            deadline_iso=None,
            seconds_remaining=None,
            urgency="none",
            reason="Bounty has reached terminal settled state with verified canonical settlement.",
            verifier_status=v_status.to_dict(),
            canonical_evidence={"has_bountysettled_event": has_settled_event, "status": STATUS_SETTLED},
        )

    # 2. Cancelled State Check
    if status == STATUS_CANCELLED or has_cancelled_event:
        return BountyItemDiagnosis(
            bounty_id=contract.bounty_id,
            contract_address=contract.contract_address,
            title=contract.title,
            current_status=STATUS_CANCELLED,
            classification=CLASS_TERMINAL,
            is_stalled=False,
            is_terminal=True,
            round=lifecycle_state.round,
            solver=lifecycle_state.solver,
            claim_expires_at=None,
            verification_expires_at=None,
            next_action=None,
            deadline=None,
            deadline_iso=None,
            seconds_remaining=None,
            urgency="none",
            reason="Bounty was cancelled on-chain. Escrow refundable.",
            verifier_status=v_status.to_dict(),
            canonical_evidence={"has_cancelled_event": has_cancelled_event, "status": STATUS_CANCELLED},
        )

    # 3. Missing Terms Check
    if not has_valid_terms_document or (contract.terms_hash in {"", "0x" + "0" * 64}):
        deadline = lifecycle_state.claim_expires_at or contract.funding_deadline or None
        rem = (deadline - now) if deadline else None
        return BountyItemDiagnosis(
            bounty_id=contract.bounty_id,
            contract_address=contract.contract_address,
            title=contract.title,
            current_status=status,
            classification=CLASS_CLAIM_EXPIRING if status == STATUS_CLAIMED else CLASS_SUBMITTED,
            is_stalled=True,
            is_terminal=False,
            round=lifecycle_state.round,
            solver=lifecycle_state.solver,
            claim_expires_at=lifecycle_state.claim_expires_at,
            verification_expires_at=lifecycle_state.verification_expires_at,
            next_action=ACTION_RESOLVE_MISSING_TERMS,
            deadline=deadline,
            deadline_iso=format_iso_timestamp(deadline),
            seconds_remaining=rem,
            urgency="high",
            reason="Bounty terms hash cannot be resolved to published canonical terms document.",
            verifier_status=v_status.to_dict(),
            canonical_evidence={"missing_terms": True, "terms_hash": contract.terms_hash},
        )

    # 4. Stale Indexer State Check
    if idx_status.is_stale or idx_status.lag_blocks > MAX_ACCEPTABLE_INDEXER_LAG_BLOCKS:
        deadline = lifecycle_state.verification_expires_at or lifecycle_state.claim_expires_at or None
        rem = (deadline - now) if deadline else None
        return BountyItemDiagnosis(
            bounty_id=contract.bounty_id,
            contract_address=contract.contract_address,
            title=contract.title,
            current_status=status,
            classification=CLASS_CLAIM_EXPIRING if status == STATUS_CLAIMED else CLASS_SUBMITTED,
            is_stalled=True,
            is_terminal=False,
            round=lifecycle_state.round,
            solver=lifecycle_state.solver,
            claim_expires_at=lifecycle_state.claim_expires_at,
            verification_expires_at=lifecycle_state.verification_expires_at,
            next_action=ACTION_REFRESH_INDEX_STATE,
            deadline=deadline,
            deadline_iso=format_iso_timestamp(deadline),
            seconds_remaining=rem,
            urgency="high",
            reason=f"Indexer sync lag ({idx_status.lag_blocks} blocks) prevents safe automated action.",
            verifier_status=v_status.to_dict(),
            canonical_evidence={"indexer_status": idx_status.to_dict()},
        )

    # 5. Submitted State Diagnoses
    if status == STATUS_SUBMITTED:
        v_deadline = lifecycle_state.verification_expires_at or 0
        rem = v_deadline - now

        # Verifier Outage / Unavailable
        if not v_status.verifiers_available or v_status.healthy_count < v_status.required_threshold:
            return BountyItemDiagnosis(
                bounty_id=contract.bounty_id,
                contract_address=contract.contract_address,
                title=contract.title,
                current_status=STATUS_SUBMITTED,
                classification=CLASS_VERIFIER_UNAVAILABLE,
                is_stalled=True,
                is_terminal=False,
                round=lifecycle_state.round,
                solver=lifecycle_state.solver,
                claim_expires_at=lifecycle_state.claim_expires_at,
                verification_expires_at=v_deadline,
                next_action=ACTION_FAILOVER_VERIFIER,
                deadline=v_deadline,
                deadline_iso=format_iso_timestamp(v_deadline),
                seconds_remaining=rem,
                urgency="high",
                reason=f"Verifier fleet outage: {v_status.error or 'insufficient healthy verifiers'}",
                verifier_status=v_status.to_dict(),
                canonical_evidence={"submission_hash": lifecycle_state.submission_hash},
            )

        # Verification Expired (Past Deadline)
        if now > v_deadline:
            return BountyItemDiagnosis(
                bounty_id=contract.bounty_id,
                contract_address=contract.contract_address,
                title=contract.title,
                current_status=STATUS_SUBMITTED,
                classification=CLASS_VERIFICATION_EXPIRING,
                is_stalled=True,
                is_terminal=False,
                round=lifecycle_state.round,
                solver=lifecycle_state.solver,
                claim_expires_at=lifecycle_state.claim_expires_at,
                verification_expires_at=v_deadline,
                next_action=ACTION_EXPIRE_SUBMISSION,
                deadline=v_deadline,
                deadline_iso=format_iso_timestamp(v_deadline),
                seconds_remaining=rem,
                urgency="critical",
                reason=f"Verification window expired {abs(rem)}s ago without verifier resolution; call expireSubmission to refund solver bond.",
                verifier_status=v_status.to_dict(),
                canonical_evidence={"submission_hash": lifecycle_state.submission_hash},
            )

        # Verification Expiring Soon (Warning Threshold)
        warning_window = min(DEFAULT_VERIFICATION_WARNING_SECONDS, contract.verification_window_seconds // 3)
        if rem <= warning_window:
            return BountyItemDiagnosis(
                bounty_id=contract.bounty_id,
                contract_address=contract.contract_address,
                title=contract.title,
                current_status=STATUS_SUBMITTED,
                classification=CLASS_VERIFICATION_EXPIRING,
                is_stalled=True,
                is_terminal=False,
                round=lifecycle_state.round,
                solver=lifecycle_state.solver,
                claim_expires_at=lifecycle_state.claim_expires_at,
                verification_expires_at=v_deadline,
                next_action=ACTION_URGENT_ATTEST_AND_SETTLE,
                deadline=v_deadline,
                deadline_iso=format_iso_timestamp(v_deadline),
                seconds_remaining=rem,
                urgency="critical" if rem <= 60 else "high",
                reason=f"Verification window expiring in {rem}s; verifiers must submit attestation quorum before deadline.",
                verifier_status=v_status.to_dict(),
                canonical_evidence={"submission_hash": lifecycle_state.submission_hash},
            )

        # Healthy Submitted
        return BountyItemDiagnosis(
            bounty_id=contract.bounty_id,
            contract_address=contract.contract_address,
            title=contract.title,
            current_status=STATUS_SUBMITTED,
            classification=CLASS_SUBMITTED,
            is_stalled=False,
            is_terminal=False,
            round=lifecycle_state.round,
            solver=lifecycle_state.solver,
            claim_expires_at=lifecycle_state.claim_expires_at,
            verification_expires_at=v_deadline,
            next_action=ACTION_ATTEST_AND_SETTLE,
            deadline=v_deadline,
            deadline_iso=format_iso_timestamp(v_deadline),
            seconds_remaining=rem,
            urgency="medium",
            reason="Submission is within active verification window with healthy verifier fleet.",
            verifier_status=v_status.to_dict(),
            canonical_evidence={"submission_hash": lifecycle_state.submission_hash},
        )

    # 6. Claimed State Diagnoses
    if status == STATUS_CLAIMED:
        c_deadline = lifecycle_state.claim_expires_at or 0
        rem = c_deadline - now

        # Claim Expired (Past Deadline)
        if now > c_deadline:
            return BountyItemDiagnosis(
                bounty_id=contract.bounty_id,
                contract_address=contract.contract_address,
                title=contract.title,
                current_status=STATUS_CLAIMED,
                classification=CLASS_CLAIM_EXPIRING,
                is_stalled=True,
                is_terminal=False,
                round=lifecycle_state.round,
                solver=lifecycle_state.solver,
                claim_expires_at=c_deadline,
                verification_expires_at=None,
                next_action=ACTION_EXPIRE_CLAIM,
                deadline=c_deadline,
                deadline_iso=format_iso_timestamp(c_deadline),
                seconds_remaining=rem,
                urgency="critical",
                reason=f"Claim window expired {abs(rem)}s ago without solver submission; trigger expireClaim to forfeit bond and reopen bounty.",
                verifier_status=v_status.to_dict(),
                canonical_evidence={"solver": lifecycle_state.solver, "claim_expires_at": c_deadline},
            )

        # Claim Expiring Soon (Warning Threshold)
        warning_window = min(DEFAULT_CLAIM_WARNING_SECONDS, contract.claim_window_seconds // 4)
        if rem <= warning_window:
            return BountyItemDiagnosis(
                bounty_id=contract.bounty_id,
                contract_address=contract.contract_address,
                title=contract.title,
                current_status=STATUS_CLAIMED,
                classification=CLASS_CLAIM_EXPIRING,
                is_stalled=True,
                is_terminal=False,
                round=lifecycle_state.round,
                solver=lifecycle_state.solver,
                claim_expires_at=c_deadline,
                verification_expires_at=None,
                next_action=ACTION_SUBMIT_WORK,
                deadline=c_deadline,
                deadline_iso=format_iso_timestamp(c_deadline),
                seconds_remaining=rem,
                urgency="critical" if rem <= 3600 else "high",
                reason=f"Claim window expiring in {rem}s; solver must submit deliverable evidence before deadline.",
                verifier_status=v_status.to_dict(),
                canonical_evidence={"solver": lifecycle_state.solver, "claim_expires_at": c_deadline},
            )

        # Healthy Claimed
        return BountyItemDiagnosis(
            bounty_id=contract.bounty_id,
            contract_address=contract.contract_address,
            title=contract.title,
            current_status=STATUS_CLAIMED,
            classification=CLASS_HEALTHY_CLAIMED,
            is_stalled=False,
            is_terminal=False,
            round=lifecycle_state.round,
            solver=lifecycle_state.solver,
            claim_expires_at=c_deadline,
            verification_expires_at=None,
            next_action=ACTION_SUBMIT_WORK,
            deadline=c_deadline,
            deadline_iso=format_iso_timestamp(c_deadline),
            seconds_remaining=rem,
            urgency="low",
            reason="Claim is actively progressing within healthy time window.",
            verifier_status=v_status.to_dict(),
            canonical_evidence={"solver": lifecycle_state.solver, "claim_expires_at": c_deadline},
        )

    # 7. Open / Claimable State
    if status == STATUS_OPEN:
        if contract.funding_deadline > 0 and now > contract.funding_deadline:
            return BountyItemDiagnosis(
                bounty_id=contract.bounty_id,
                contract_address=contract.contract_address,
                title=contract.title,
                current_status=STATUS_OPEN,
                classification=CLASS_TERMINAL,
                is_stalled=True,
                is_terminal=False,
                round=0,
                solver=None,
                claim_expires_at=None,
                verification_expires_at=None,
                next_action=ACTION_CANCEL_UNFUNDED,
                deadline=contract.funding_deadline,
                deadline_iso=format_iso_timestamp(contract.funding_deadline),
                seconds_remaining=contract.funding_deadline - now,
                urgency="medium",
                reason="Bounty funding deadline has passed without full escrow funding.",
                verifier_status=v_status.to_dict(),
                canonical_evidence={"funding_deadline": contract.funding_deadline},
            )

    return BountyItemDiagnosis(
        bounty_id=contract.bounty_id,
        contract_address=contract.contract_address,
        title=contract.title,
        current_status=status,
        classification=status if status in DIAGNOSTIC_CLASSIFICATIONS else CLASS_TERMINAL,
        is_stalled=False,
        is_terminal=False,
        round=lifecycle_state.round,
        solver=lifecycle_state.solver,
        claim_expires_at=None,
        verification_expires_at=None,
        next_action=None,
        deadline=None,
        deadline_iso=None,
        seconds_remaining=None,
        urgency="low",
        reason=f"Bounty in {status} state.",
        verifier_status=v_status.to_dict(),
        canonical_evidence={},
    )


def diagnose_backlog(
    cases: Sequence[Mapping[str, Any]],
    reference_timestamp: int | None = None,
) -> DiagnosticsReport:
    """Run diagnostics across a list of bounty cases / contracts."""
    now = int(reference_timestamp if reference_timestamp is not None else time.time())
    items: list[BountyItemDiagnosis] = []

    for raw_case in cases:
        raw_c = raw_case.get("contract", raw_case)
        contract = BountyContractConfig(
            bounty_id=str(raw_c.get("bounty_id", "")),
            contract_address=str(raw_c.get("contract_address", "")),
            title=str(raw_c.get("title", "")),
            creator=str(raw_c.get("creator", "")),
            settlement_token=str(raw_c.get("settlement_token", "")),
            solver_reward=int(raw_c.get("solver_reward", 0)),
            verifier_reward=int(raw_c.get("verifier_reward", 0)),
            claim_bond=int(raw_c.get("claim_bond", 0)),
            funding_deadline=int(raw_c.get("funding_deadline", 0)),
            claim_window_seconds=int(raw_c.get("claim_window_seconds", 604800)),
            verification_window_seconds=int(raw_c.get("verification_window_seconds", 7200)),
            terms_hash=str(raw_c.get("terms_hash", "")),
            policy_hash=str(raw_c.get("policy_hash", "")),
            verification_mode=str(raw_c.get("verification_mode", "signed_quorum")),
            verifier_module=str(raw_c.get("verifier_module", "")),
            verifiers=tuple(raw_c.get("verifiers", ())),
            threshold=int(raw_c.get("threshold", 1)),
        )

        raw_state = raw_case.get("lifecycle_state", raw_case)
        state = BountyLifecycleState(
            status=str(raw_state.get("status", STATUS_OPEN)),
            round=int(raw_state.get("round", 0)),
            solver=raw_state.get("solver"),
            claim_expires_at=raw_state.get("claim_expires_at"),
            verification_expires_at=raw_state.get("verification_expires_at"),
            submission_hash=raw_state.get("submission_hash"),
            evidence_hash=raw_state.get("evidence_hash"),
            active_claim_bond=int(raw_state.get("active_claim_bond", 0)),
            timeout_bond_pool=int(raw_state.get("timeout_bond_pool", 0)),
        )

        events = parse_canonical_events(raw_case.get("canonical_events", ()))

        raw_v = raw_case.get("verifier_status", {})
        v_status = VerifierFleetStatus(
            verifiers_available=bool(raw_v.get("verifiers_available", True)),
            healthy_count=int(raw_v.get("healthy_count", 2)),
            required_threshold=int(raw_v.get("required_threshold", 2)),
            error=raw_v.get("error"),
        )

        raw_idx = raw_case.get("indexer_status", {})
        idx_status = IndexerSyncStatus(
            is_stale=bool(raw_idx.get("is_stale", False)),
            lag_blocks=int(raw_idx.get("lag_blocks", 0)),
            heartbeat_age_seconds=int(raw_idx.get("heartbeat_age_seconds", 0)),
        )

        has_valid_terms = bool(raw_case.get("has_valid_terms_document", True))
        case_ref_time = raw_case.get("reference_timestamp", now)

        diagnosis = diagnose_bounty(
            contract=contract,
            lifecycle_state=state,
            canonical_events=events,
            verifier_status=v_status,
            indexer_status=idx_status,
            has_valid_terms_document=has_valid_terms,
            reference_timestamp=case_ref_time,
        )
        items.append(diagnosis)

    # Calculate backlog and summary metrics
    summary = {
        "total_bounties": len(items),
        "healthy_claimed": sum(1 for i in items if i.classification == CLASS_HEALTHY_CLAIMED),
        "claim_expiring": sum(1 for i in items if i.classification == CLASS_CLAIM_EXPIRING),
        "submitted": sum(1 for i in items if i.classification == CLASS_SUBMITTED),
        "verification_expiring": sum(1 for i in items if i.classification == CLASS_VERIFICATION_EXPIRING),
        "verifier_unavailable": sum(1 for i in items if i.classification == CLASS_VERIFIER_UNAVAILABLE),
        "settled": sum(1 for i in items if i.classification == CLASS_SETTLED),
        "terminal": sum(1 for i in items if i.is_terminal),
        "stalled_backlog": sum(1 for i in items if i.is_stalled),
        "urgent_actions": sum(1 for i in items if i.urgency in {"critical", "high"}),
    }

    # Stalled backlog items sorted by deadline (closest deadline first)
    backlog = sorted(
        [item for item in items if item.is_stalled],
        key=lambda x: (x.deadline if x.deadline is not None else 9999999999),
    )

    observed_at = datetime.fromtimestamp(now, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    operations_markdown = generate_operations_markdown(summary, backlog, items, observed_at)

    return DiagnosticsReport(
        schema_version=SCHEMA_VERSION,
        observed_at=observed_at,
        reference_timestamp=now,
        summary=summary,
        items=items,
        backlog=backlog,
        operations_markdown=operations_markdown,
    )


def generate_operations_markdown(
    summary: Mapping[str, int],
    backlog: Sequence[BountyItemDiagnosis],
    all_items: Sequence[BountyItemDiagnosis],
    observed_at: str,
) -> str:
    """Generate formatted markdown runbook for operations engineers and keepers."""
    lines: list[str] = [
        "# Stalled Bounty Operations & Backlog Diagnostic Report",
        "",
        f"**Observed At:** `{observed_at}` | **Schema:** `{SCHEMA_VERSION}`",
        "",
        "## Executive Summary",
        "",
        "| Metric | Count | Status |",
        "| :--- | :--- | :--- |",
        f"| Total Tracked Bounties | {summary.get('total_bounties', 0)} | ℹ️ |",
        f"| Stalled Backlog Items | {summary.get('stalled_backlog', 0)} | {'🚨 Action Required' if summary.get('stalled_backlog', 0) > 0 else '✅ Clear'} |",
        f"| Urgent Actions (Critical/High) | {summary.get('urgent_actions', 0)} | {'⚠️ Attention Needed' if summary.get('urgent_actions', 0) > 0 else '✅ None'} |",
        f"| Healthy Claimed | {summary.get('healthy_claimed', 0)} | 🟢 Active |",
        f"| Claim Expiring / Stalled | {summary.get('claim_expiring', 0)} | 🟡 Impending |",
        f"| Healthy Submitted | {summary.get('submitted', 0)} | 🔵 In Review |",
        f"| Verification Expiring | {summary.get('verification_expiring', 0)} | 🟠 Escalated |",
        f"| Verifier Unavailable / Outage | {summary.get('verifier_unavailable', 0)} | 🔴 Infra Failure |",
        f"| Settled (BountySettled) | {summary.get('settled', 0)} | 🏁 Finalized |",
        "",
    ]

    if backlog:
        lines.extend([
            "## Prioritized Stalled Work Backlog",
            "",
            "| Bounty ID | Status | Classification | Urgency | Next Action | Deadline | Reason |",
            "| :--- | :--- | :--- | :--- | :--- | :--- | :--- |",
        ])
        for b in backlog:
            b_id_short = b.bounty_id[:10] + "..." if len(b.bounty_id) > 10 else b.bounty_id
            deadline_str = b.deadline_iso or str(b.deadline) or "N/A"
            lines.append(
                f"| `{b_id_short}` | `{b.current_status}` | `{b.classification}` | **{b.urgency.upper()}** | `{b.next_action}` | `{deadline_str}` | {b.reason} |"
            )
        lines.append("")
    else:
        lines.extend([
            "## Stalled Work Backlog",
            "",
            "> [!NOTE]",
            "> **No stalled items.** All active claimed and submitted bounties are healthy within operational windows.",
            "",
        ])

    return "\n".join(lines)


def to_discovery_projection(report: DiagnosticsReport) -> dict[str, Any]:
    """Format diagnostic report into machine-readable discovery projection."""
    return {
        "schema": DISCOVERY_SCHEMA,
        "observed_at": report.observed_at,
        "reference_timestamp": report.reference_timestamp,
        "summary": report.summary,
        "backlog": [
            {
                "bounty_id": item.bounty_id,
                "contract_address": item.contract_address,
                "title": item.title,
                "status": item.current_status,
                "classification": item.classification,
                "next_action": item.next_action,
                "deadline": item.deadline,
                "deadline_iso": item.deadline_iso,
                "seconds_remaining": item.seconds_remaining,
                "urgency": item.urgency,
            }
            for item in report.backlog
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Diagnose stalled claimed and submitted bounties with deterministic recovery actions."
    )
    parser.add_argument(
        "--fixtures",
        type=Path,
        default=Path("ops/fixtures/stalled-bounty-cases.json"),
        help="Path to fixtures JSON file.",
    )
    parser.add_argument(
        "--format",
        choices=["json", "markdown", "operations", "discovery"],
        default="json",
        help="Output format style.",
    )
    parser.add_argument(
        "--reference-timestamp",
        type=int,
        default=None,
        help="Explicit reference timestamp for reproducible testing.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Optional file destination to write output.",
    )
    parser.add_argument(
        "--fail-on-stalled",
        action="store_true",
        help="Exit with non-zero status if any stalled items are found.",
    )

    args = parser.parse_args()

    if not args.fixtures.is_file():
        print(f"Error: fixtures file not found: {args.fixtures}", file=sys.stderr)
        return 1

    try:
        data = json.loads(args.fixtures.read_text(encoding="utf-8"))
    except Exception as err:
        print(f"Error reading fixtures JSON: {err}", file=sys.stderr)
        return 1

    cases = data.get("cases", [data] if "contract" in data else [])
    ref_time = args.reference_timestamp or data.get("reference_timestamp")

    report = diagnose_backlog(cases, reference_timestamp=ref_time)

    if args.format == "json":
        out_text = json.dumps(report.to_dict(), indent=2)
    elif args.format in {"markdown", "operations"}:
        out_text = report.operations_markdown
    elif args.format == "discovery":
        out_text = json.dumps(to_discovery_projection(report), indent=2)
    else:
        out_text = json.dumps(report.to_dict(), indent=2)

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(out_text + "\n", encoding="utf-8")
    else:
        print(out_text)

    if args.fail_on_stalled and report.summary["stalled_backlog"] > 0:
        return 2

    return 0


if __name__ == "__main__":
    sys.exit(main())
