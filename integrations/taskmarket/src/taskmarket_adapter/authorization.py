"""Operator-issued, action-scoped authorization artifacts.

Write operations must never trust a caller-supplied boolean. Instead, the host
operator issues a signed artifact out of band:

  * `TASKMARKET_AUTHORIZATION_FILE` - path to the artifact JSON file.
  * `TASKMARKET_OPERATOR_SECRET`    - shared secret; the operator signs the
    payload with HMAC-SHA256 and only the host process knows the key.

Artifact format (JSON object):

    {
      "version": 1,
      "action": "task_create" | "task_submit",
      "expires_at": "2026-01-01T00:00:00+00:00",   // aware ISO-8601 UTC
      "max_reward_usdc": "10",                     // optional, task_create
      "max_duration_hours": 168,                   // optional, task_create
      "task_id": "<0x..>",                         // optional, task_submit bind
      "signature": "<hex hmac-sha256 of canonical payload>"
    }

Every check fails closed: missing env vars, unreadable or malformed files,
unknown versions/actions, expired timestamps, bad signatures, or caps that do
not cover the request all refuse the operation.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import os
import pathlib
from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any, Mapping, Optional

from .errors import AuthorizationError
from .units import parse_usdc

ARTIFACT_VERSION = 1
ACTION_TASK_CREATE = "task_create"
ACTION_TASK_SUBMIT = "task_submit"
KNOWN_ACTIONS = frozenset({ACTION_TASK_CREATE, ACTION_TASK_SUBMIT})

ENV_AUTHORIZATION_FILE = "TASKMARKET_AUTHORIZATION_FILE"
ENV_OPERATOR_SECRET = "TASKMARKET_OPERATOR_SECRET"


@dataclass(frozen=True)
class AuthorizationSpec:
    action: str
    expires_at: datetime
    max_reward_usdc: Optional[Decimal] = None
    max_duration_hours: Optional[int] = None
    task_id: Optional[str] = None


def _canonical_payload(payload: Mapping[str, Any]) -> bytes:
    return json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")


def sign_artifact(payload: Mapping[str, Any], secret: str) -> str:
    """Helper for operators/tests: HMAC-SHA256 hex digest over the canonical payload."""
    return hmac.new(secret.encode("utf-8"), _canonical_payload(payload), hashlib.sha256).hexdigest()


def write_artifact(path: pathlib.Path, spec: dict[str, Any], secret: str) -> None:
    """Helper for operators/tests: build and persist a correctly signed artifact."""
    body = {key: value for key, value in spec.items() if key != "signature"}
    body["signature"] = sign_artifact(body, secret)
    path.write_text(json.dumps(body), encoding="utf-8")


def _parse_expires_at(value: Any) -> datetime:
    if not isinstance(value, str):
        raise AuthorizationError("authorization refused: invalid expiry")
    text = value.strip()
    if text.endswith(("Z", "z")):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        raise AuthorizationError("authorization refused: invalid expiry") from None
    if parsed.tzinfo is None:
        raise AuthorizationError("authorization refused: expiry must include a timezone")
    return parsed.astimezone(timezone.utc)


def load_authorization(
    required_action: str,
    *,
    env: Optional[Mapping[str, str]] = None,
    now: Optional[datetime] = None,
) -> AuthorizationSpec:
    """Load and fully verify an operator artifact. Fails closed on any problem."""
    if required_action not in KNOWN_ACTIONS:
        raise AuthorizationError("authorization refused: unknown action")
    environment = os.environ if env is None else env

    secret = environment.get(ENV_OPERATOR_SECRET)
    if not secret:
        raise AuthorizationError("authorization refused: no operator secret configured")

    artifact_path = environment.get(ENV_AUTHORIZATION_FILE)
    if not artifact_path:
        raise AuthorizationError("authorization refused: no authorization artifact configured")
    try:
        raw_text = pathlib.Path(artifact_path).read_text(encoding="utf-8")
    except OSError:
        raise AuthorizationError("authorization refused: authorization artifact unreadable") from None

    try:
        document = json.loads(raw_text)
    except json.JSONDecodeError:
        raise AuthorizationError("authorization refused: malformed authorization artifact") from None
    if not isinstance(document, dict):
        raise AuthorizationError("authorization refused: malformed authorization artifact")

    signature = document.get("signature")
    if not isinstance(signature, str):
        raise AuthorizationError("authorization refused: unsigned authorization artifact")
    expected = sign_artifact({k: v for k, v in document.items() if k != "signature"}, secret)
    if not hmac.compare_digest(signature, expected):
        raise AuthorizationError("authorization refused: signature verification failed")

    if document.get("version") != ARTIFACT_VERSION:
        raise AuthorizationError("authorization refused: unsupported artifact version")

    action = document.get("action")
    if action != required_action:
        raise AuthorizationError(
            f"authorization refused: artifact does not cover action '{required_action}'"
        )

    expires_at = _parse_expires_at(document.get("expires_at"))
    current = now if now is not None else datetime.now(timezone.utc)
    if current >= expires_at:
        raise AuthorizationError("authorization refused: authorization expired")

    max_reward_raw = document.get("max_reward_usdc")
    max_reward = parse_usdc(max_reward_raw) if max_reward_raw is not None else None

    max_duration_raw = document.get("max_duration_hours")
    if max_duration_raw is not None:
        if isinstance(max_duration_raw, bool) or not isinstance(max_duration_raw, int) or max_duration_raw <= 0:
            raise AuthorizationError("authorization refused: invalid duration cap in artifact")
        max_duration: Optional[int] = max_duration_raw
    else:
        max_duration = None

    bound_task_id = document.get("task_id")
    if bound_task_id is not None and not isinstance(bound_task_id, str):
        raise AuthorizationError("authorization refused: invalid task binding in artifact")

    return AuthorizationSpec(
        action=action,
        expires_at=expires_at,
        max_reward_usdc=max_reward,
        max_duration_hours=max_duration,
        task_id=bound_task_id,
    )


def enforce_spend(spec: AuthorizationSpec, reward_usdc: Decimal, duration_hours: int) -> None:
    """Check a requested create against the artifact's spend/time caps."""
    if spec.max_reward_usdc is not None and reward_usdc > spec.max_reward_usdc:
        raise AuthorizationError("authorization refused: requested reward exceeds authorized cap")
    if spec.max_duration_hours is not None and duration_hours > spec.max_duration_hours:
        raise AuthorizationError("authorization refused: requested duration exceeds authorized cap")


def enforce_task_binding(spec: AuthorizationSpec, task_id: str) -> None:
    """If the submit artifact is bound to one task, the id must match exactly."""
    if spec.task_id is not None and spec.task_id != task_id:
        raise AuthorizationError("authorization refused: artifact is bound to a different task")
