#!/usr/bin/env python3
"""Serve the local Solarpunk site with bounded OAuth callbacks.

This server is intentionally local-only. It keeps provider credentials in the
ignored .env.auth.local file, never exposes provider access tokens to browser
JavaScript, and issues a short-lived signed HttpOnly session cookie.
"""

from __future__ import annotations

import argparse
import base64
from datetime import datetime, timezone
from decimal import Decimal
import hashlib
import hmac
import json
import os
import re
import secrets
import sys
import threading
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from http import HTTPStatus
from http.cookies import SimpleCookie
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

try:
    from eth_account import Account
    from eth_account.messages import encode_defunct
except ImportError:  # pragma: no cover - exercised by the explicit unavailable branch
    Account = None
    encode_defunct = None


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SITE = ROOT / "site"
DEFAULT_ENV = ROOT / ".env.auth.local"
DEFAULT_WALLET_LINKS = ROOT / ".wallet-links.local.json"
LOOPBACK_HOSTS = {"127.0.0.1", "localhost", "::1"}
SESSION_COOKIE = "agent_bounties_session"
PASSWORD_SETUP_COOKIE = "agent_bounties_password_setup"
SESSION_MAX_AGE = 8 * 60 * 60
STATE_MAX_AGE = 10 * 60
WALLET_CHALLENGE_MAX_AGE = 5 * 60
BASE_CHAIN_ID = 8453
EVM_ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
EVM_SIGNATURE_RE = re.compile(r"^0x[0-9a-fA-F]{130}$")

OAUTH_STATES: dict[str, dict[str, Any]] = {}
STATE_LOCK = threading.Lock()
WALLET_CHALLENGES: dict[str, dict[str, Any]] = {}
WALLET_CHALLENGE_LOCK = threading.Lock()
PUBLIC_EVIDENCE_CACHE: dict[str, Any] = {"loaded_at": 0.0, "payload": None}
PUBLIC_EVIDENCE_LOCK = threading.Lock()


def load_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.exists():
        return values
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    return values


def b64url_encode(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).decode("ascii").rstrip("=")


def b64url_decode(value: str) -> bytes:
    return base64.urlsafe_b64decode(value + ("=" * (-len(value) % 4)))


def sign_session(user: dict[str, Any], secret: str, now: int | None = None) -> str:
    issued_at = int(time.time() if now is None else now)
    safe_user = {
        "provider": str(user.get("provider", "")),
        "sub": str(user.get("sub", "")),
        "name": str(user.get("name", ""))[:160],
        "email": str(user.get("email", ""))[:320],
        "avatar": str(user.get("avatar", ""))[:2048],
        "iat": issued_at,
        "exp": issued_at + SESSION_MAX_AGE,
        "credential_version": int(user.get("credential_version", 0)),
    }
    payload = b64url_encode(json.dumps(safe_user, separators=(",", ":"), sort_keys=True).encode("utf-8"))
    signature = b64url_encode(hmac.new(secret.encode("utf-8"), payload.encode("ascii"), hashlib.sha256).digest())
    return f"{payload}.{signature}"


def verify_session(token: str, secret: str, now: int | None = None) -> dict[str, Any] | None:
    try:
        payload, supplied_signature = token.split(".", 1)
        expected = b64url_encode(hmac.new(secret.encode("utf-8"), payload.encode("ascii"), hashlib.sha256).digest())
        if not hmac.compare_digest(supplied_signature, expected):
            return None
        user = json.loads(b64url_decode(payload))
        current = int(time.time() if now is None else now)
        if not isinstance(user, dict) or current >= int(user.get("exp", 0)):
            return None
        if not user.get("provider") or not user.get("sub"):
            return None
        return user
    except (ValueError, TypeError, json.JSONDecodeError):
        return None


def normalize_wallet_address(value: Any) -> str:
    address = str(value or "").strip()
    if not EVM_ADDRESS_RE.fullmatch(address):
        raise ValueError("invalid_wallet_address")
    return address.lower()


def account_identifier(user: dict[str, Any], secret: str) -> str:
    subject = f"{user.get('provider', '')}\0{user.get('sub', '')}".encode("utf-8")
    return hmac.new(secret.encode("utf-8"), subject, hashlib.sha256).hexdigest()


def utc_iso(timestamp: int | float) -> str:
    return datetime.fromtimestamp(timestamp, tz=timezone.utc).isoformat().replace("+00:00", "Z")


def wallet_link_message(
    origin: str,
    account_id: str,
    address: str,
    nonce: str,
    issued_at: int,
    expires_at: int,
) -> str:
    return "\n".join(
        (
            "Agent Bounties wallet ownership verification",
            "",
            "Sign this message to link the wallet to your signed-in Agent Bounties account.",
            "This proves address control only. It does not authorize a transaction, token approval, or payment.",
            "",
            f"Origin: {origin}",
            f"Account: {account_id}",
            f"Wallet: {address}",
            f"Chain ID: {BASE_CHAIN_ID}",
            f"Nonce: {nonce}",
            f"Issued At: {utc_iso(issued_at)}",
            f"Expiration Time: {utc_iso(expires_at)}",
        )
    )


def verify_wallet_signature(message: str, signature: Any, expected_address: str) -> bool:
    value = str(signature or "").strip()
    if not EVM_SIGNATURE_RE.fullmatch(value) or Account is None or encode_defunct is None:
        return False
    try:
        recovered = Account.recover_message(encode_defunct(text=message), signature=value)
    except (ValueError, TypeError):
        return False
    return normalize_wallet_address(recovered) == normalize_wallet_address(expected_address)


class WalletLinkStore:
    def __init__(self, path: Path, secret: str):
        self.path = path
        self.secret = secret
        self.lock = threading.Lock()

    def account_id(self, user: dict[str, Any]) -> str:
        return account_identifier(user, self.secret)

    @staticmethod
    def empty() -> dict[str, Any]:
        return {
            "schema_version": "agent-bounties/local-wallet-links-v1",
            "accounts": {},
            "owners": {},
        }

    def _read_unlocked(self) -> dict[str, Any]:
        if not self.path.exists():
            return self.empty()
        payload = json.loads(self.path.read_text(encoding="utf-8"))
        if (
            not isinstance(payload, dict)
            or payload.get("schema_version") != "agent-bounties/local-wallet-links-v1"
            or not isinstance(payload.get("accounts"), dict)
            or not isinstance(payload.get("owners"), dict)
        ):
            raise ValueError("wallet_link_store_invalid")
        return payload

    def _write_unlocked(self, payload: dict[str, Any]) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self.path.with_name(f"{self.path.name}.{secrets.token_hex(6)}.tmp")
        temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        try:
            os.chmod(temporary, 0o600)
        except OSError:
            pass
        os.replace(temporary, self.path)

    def wallets_for(self, user: dict[str, Any]) -> list[dict[str, Any]]:
        key = self.account_id(user)
        with self.lock:
            payload = self._read_unlocked()
            account = payload["accounts"].get(key, {})
            wallets = account.get("wallets", []) if isinstance(account, dict) else []
            if not isinstance(wallets, list):
                raise ValueError("wallet_link_store_invalid")
            return [dict(wallet) for wallet in wallets if isinstance(wallet, dict)]

    def link(self, user: dict[str, Any], address: str, linked_at: str) -> list[dict[str, Any]]:
        normalized = normalize_wallet_address(address)
        key = self.account_id(user)
        with self.lock:
            payload = self._read_unlocked()
            owner = payload["owners"].get(normalized)
            if owner and owner != key:
                raise ValueError("wallet_linked_to_another_account")
            account = payload["accounts"].setdefault(key, {"wallets": []})
            wallets = account.setdefault("wallets", [])
            if not isinstance(wallets, list):
                raise ValueError("wallet_link_store_invalid")
            if not any(wallet.get("address") == normalized for wallet in wallets if isinstance(wallet, dict)):
                if len(wallets) >= 8:
                    raise ValueError("wallet_limit_reached")
                wallets.append(
                    {
                        "address": normalized,
                        "chain_id": BASE_CHAIN_ID,
                        "linked_at": linked_at,
                        "proof": "eip191_personal_sign",
                    }
                )
            payload["owners"][normalized] = key
            self._write_unlocked(payload)
            return [dict(wallet) for wallet in wallets]

    def unlink(self, user: dict[str, Any], address: str) -> list[dict[str, Any]]:
        normalized = normalize_wallet_address(address)
        key = self.account_id(user)
        with self.lock:
            payload = self._read_unlocked()
            account = payload["accounts"].get(key)
            if not isinstance(account, dict) or not isinstance(account.get("wallets"), list):
                return []
            wallets = [
                wallet
                for wallet in account["wallets"]
                if isinstance(wallet, dict) and wallet.get("address") != normalized
            ]
            account["wallets"] = wallets
            if payload["owners"].get(normalized) == key:
                payload["owners"].pop(normalized, None)
            if not wallets:
                payload["accounts"].pop(key, None)
            self._write_unlocked(payload)
            return [dict(wallet) for wallet in wallets]


def unavailable_account_dashboard(reason: str, wallets: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    linked_wallets = wallets or []
    return {
        "schema_version": "agent-bounties/local-account-dashboard-v1",
        "data_status": "unavailable",
        "reason": reason,
        "identity_link_status": "verified" if linked_wallets else "unlinked",
        "wallets": linked_wallets,
        "stats": {
            "participating_bounties": None,
            "completed_posted_bounties": None,
            "earned_usdc": None,
            "spent_usdc": None,
            "leaderboard_rank": None,
        },
        "activities": {
            "participating": [],
            "completed_posts": [],
        },
        "evidence_boundary": (
            "OAuth authentication alone does not prove ownership of a marketplace wallet. "
            "Personal values are shown only after address control is verified and every required canonical evidence source loads."
        ),
    }


def provider_config(env: dict[str, str], provider: str) -> dict[str, str] | None:
    origin = env.get("AUTH_ORIGIN", "http://127.0.0.1:4173").rstrip("/")
    if provider == "google":
        return {
            "client_id": env.get("GOOGLE_OAUTH_CLIENT_ID", ""),
            "client_secret": env.get("GOOGLE_OAUTH_CLIENT_SECRET", ""),
            "redirect_uri": env.get("GOOGLE_OAUTH_REDIRECT_URI", f"{origin}/auth/callback/google"),
            "authorize_url": "https://accounts.google.com/o/oauth2/v2/auth",
            "token_url": "https://oauth2.googleapis.com/token",
            "user_url": "https://openidconnect.googleapis.com/v1/userinfo",
        }
    if provider == "github":
        return {
            "client_id": env.get("GITHUB_OAUTH_CLIENT_ID", ""),
            "client_secret": env.get("GITHUB_OAUTH_CLIENT_SECRET", ""),
            "redirect_uri": env.get("GITHUB_OAUTH_REDIRECT_URI", f"{origin}/auth/callback/github"),
            "authorize_url": "https://github.com/login/oauth/authorize",
            "token_url": "https://github.com/login/oauth/access_token",
            "user_url": "https://api.github.com/user",
        }
    if provider == "microsoft":
        tenant = env.get("MICROSOFT_OAUTH_TENANT", "common").strip() or "common"
        authority = f"https://login.microsoftonline.com/{urllib.parse.quote(tenant, safe='')}"
        return {
            "client_id": env.get("MICROSOFT_OAUTH_CLIENT_ID", ""),
            "client_secret": env.get("MICROSOFT_OAUTH_CLIENT_SECRET", ""),
            "redirect_uri": env.get("MICROSOFT_OAUTH_REDIRECT_URI", f"{origin}/auth/callback/microsoft"),
            "authorize_url": f"{authority}/oauth2/v2.0/authorize",
            "token_url": f"{authority}/oauth2/v2.0/token",
            "user_url": "https://graph.microsoft.com/oidc/userinfo",
        }
    if provider == "amazon":
        return {
            "client_id": env.get("AMAZON_OAUTH_CLIENT_ID", ""),
            "client_secret": env.get("AMAZON_OAUTH_CLIENT_SECRET", ""),
            "redirect_uri": env.get("AMAZON_OAUTH_REDIRECT_URI", f"{origin}/auth/callback/amazon"),
            "authorize_url": "https://www.amazon.com/ap/oa",
            "token_url": "https://api.amazon.com/auth/o2/token",
            "user_url": "https://api.amazon.com/user/profile",
        }
    return None


def configured_providers(env: dict[str, str]) -> dict[str, bool]:
    return {
        "google": bool(env.get("GOOGLE_OAUTH_CLIENT_ID") and env.get("GOOGLE_OAUTH_CLIENT_SECRET")),
        "github": bool(env.get("GITHUB_OAUTH_CLIENT_ID") and env.get("GITHUB_OAUTH_CLIENT_SECRET")),
        "microsoft": bool(env.get("MICROSOFT_OAUTH_CLIENT_ID") and env.get("MICROSOFT_OAUTH_CLIENT_SECRET")),
        "amazon": bool(env.get("AMAZON_OAUTH_CLIENT_ID") and env.get("AMAZON_OAUTH_CLIENT_SECRET")),
        "enterprise": bool(env.get("ENTERPRISE_OIDC_CLIENT_ID") and env.get("ENTERPRISE_OIDC_CLIENT_SECRET")),
    }


def authorization_url(provider: str, config: dict[str, str], state: str) -> str:
    params: dict[str, str] = {
        "client_id": config["client_id"],
        "redirect_uri": config["redirect_uri"],
        "response_type": "code",
        "state": state,
    }
    if provider == "google":
        params.update({
            "scope": "openid email profile",
            "include_granted_scopes": "true",
            "prompt": "select_account",
        })
    elif provider == "github":
        params.update({"scope": "read:user user:email", "allow_signup": "true"})
    elif provider == "microsoft":
        params.update({"scope": "openid profile email", "response_mode": "query", "prompt": "select_account"})
    elif provider == "amazon":
        params.update({"scope": "profile"})
    return f"{config['authorize_url']}?{urllib.parse.urlencode(params)}"


def request_json(
    url: str,
    *,
    method: str = "GET",
    data: dict[str, str] | None = None,
    headers: dict[str, str] | None = None,
) -> Any:
    encoded = urllib.parse.urlencode(data).encode("utf-8") if data is not None else None
    request_headers = {"Accept": "application/json", "User-Agent": "AgentBounties-LocalAuth/1.0"}
    request_headers.update(headers or {})
    request = urllib.request.Request(url, data=encoded, headers=request_headers, method=method)
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.loads(response.read().decode("utf-8"))


def public_account_evidence(api_base_url: str, now: float | None = None) -> dict[str, Any]:
    current = time.time() if now is None else now
    api = api_base_url.rstrip("/")
    with PUBLIC_EVIDENCE_LOCK:
        cached = PUBLIC_EVIDENCE_CACHE.get("payload")
        if (
            isinstance(cached, dict)
            and PUBLIC_EVIDENCE_CACHE.get("api_base_url") == api
            and current - float(PUBLIC_EVIDENCE_CACHE.get("loaded_at", 0)) < 60
        ):
            return cached

    payload = {
        "autonomous": request_json(f"{api}/v1/base/autonomous-bounties/events?network=base-mainnet"),
        "competition_v1": request_json(f"{api}/v1/base/open-competition-v1/events?network=base-mainnet"),
        "competition_v2": request_json(f"{api}/v1/base/open-competition-v2-beta3/events?network=base-mainnet"),
        "leaderboard": request_json(f"{api}/v1/base/autonomous-bounties/leaderboard?network=base-mainnet"),
    }
    with PUBLIC_EVIDENCE_LOCK:
        PUBLIC_EVIDENCE_CACHE.update(
            {"loaded_at": current, "api_base_url": api, "payload": payload}
        )
    return payload


def event_list(payload: Any) -> list[dict[str, Any]]:
    events = payload if isinstance(payload, list) else payload.get("events") if isinstance(payload, dict) else None
    if not isinstance(events, list) or not all(isinstance(event, dict) for event in events):
        raise ValueError("canonical_event_stream_invalid")
    return events


def bounty_label(bounty_id: Any) -> str:
    value = str(bounty_id or "")
    if value.startswith("0x") and len(value) >= 14:
        return f"Bounty {value[:8]}…{value[-4:]}"
    return "Canonical bounty"


def usdc_from_base_units(value: int) -> str:
    return format(Decimal(value) / Decimal(1_000_000), "f")


def build_linked_account_dashboard(
    wallets: list[dict[str, Any]],
    evidence: dict[str, Any],
) -> dict[str, Any]:
    addresses = {normalize_wallet_address(wallet.get("address")) for wallet in wallets}
    if not addresses:
        return unavailable_account_dashboard("marketplace_identity_unlinked")

    autonomous = event_list(evidence.get("autonomous"))
    competition_v1 = event_list(evidence.get("competition_v1"))
    competition_v2 = event_list(evidence.get("competition_v2"))
    leaderboard = evidence.get("leaderboard")
    if not isinstance(leaderboard, dict):
        raise ValueError("leaderboard_evidence_invalid")

    earned_base_units = 0
    spent_base_units = 0
    streams = (
        ("autonomous", autonomous, "bounty_settled"),
        ("competition_v1", competition_v1, "bounty_settled"),
        ("competition_v2", competition_v2, "competition_settled"),
    )
    for _source, events, settled_kind in streams:
        for event in events:
            data = event.get("data") if isinstance(event.get("data"), dict) else {}
            if event.get("kind") == "funding_added" and str(data.get("contributor", "")).lower() in addresses:
                amount = data.get("amount")
                if not isinstance(amount, int) or amount < 0:
                    raise ValueError("funding_evidence_invalid")
                spent_base_units += amount
            if event.get("kind") == settled_kind and str(data.get("solver", "")).lower() in addresses:
                reward = data.get("solver_reward")
                bonus = data.get("timeout_bond_bonus", 0)
                if not isinstance(reward, int) or reward < 0 or not isinstance(bonus, int) or bonus < 0:
                    raise ValueError("settlement_evidence_invalid")
                earned_base_units += reward + bonus

    terminal_rounds: set[tuple[str, int]] = set()
    for event in autonomous:
        if event.get("kind") not in {
            "bounty_settled",
            "claim_expired",
            "submission_expired",
            "submission_rejected",
        }:
            continue
        data = event.get("data") if isinstance(event.get("data"), dict) else {}
        round_number = data.get("round")
        if isinstance(round_number, int):
            terminal_rounds.add((str(event.get("bounty_id", "")), round_number))

    participating: dict[tuple[str, str], dict[str, str]] = {}
    for event in autonomous:
        if event.get("kind") != "bounty_claimed":
            continue
        data = event.get("data") if isinstance(event.get("data"), dict) else {}
        if str(data.get("solver", "")).lower() not in addresses or not isinstance(data.get("round"), int):
            continue
        bounty_id = str(event.get("bounty_id", ""))
        if (bounty_id, data["round"]) in terminal_rounds:
            continue
        participating[("autonomous", bounty_id)] = {
            "title": bounty_label(bounty_id),
            "status": "Claim active",
            "occurred_at": str(event.get("occurred_at", "")),
        }

    for source, events, entry_kind, settled_kind, status in (
        ("competition_v1", competition_v1, "solution_committed", "bounty_settled", "Entry committed"),
        ("competition_v2", competition_v2, "entry_qualified", "competition_settled", "Qualified entry"),
    ):
        settled = {str(event.get("bounty_id", "")) for event in events if event.get("kind") == settled_kind}
        for event in events:
            data = event.get("data") if isinstance(event.get("data"), dict) else {}
            bounty_id = str(event.get("bounty_id", ""))
            if (
                event.get("kind") == entry_kind
                and str(data.get("solver", "")).lower() in addresses
                and bounty_id not in settled
            ):
                participating[(source, bounty_id)] = {
                    "title": bounty_label(bounty_id),
                    "status": status,
                    "occurred_at": str(event.get("occurred_at", "")),
                }

    completed_posts: dict[tuple[str, str], dict[str, str]] = {}
    for source, events, settled_kind in streams:
        created: set[str] = set()
        settlements: dict[str, str] = {}
        for event in events:
            data = event.get("data") if isinstance(event.get("data"), dict) else {}
            bounty_id = str(event.get("bounty_id", ""))
            if event.get("kind") in {"canonical_bounty_created", "canonical_competition_created"}:
                if str(data.get("creator", "")).lower() in addresses:
                    created.add(bounty_id)
            if event.get("kind") == settled_kind:
                settlements[bounty_id] = str(event.get("occurred_at", ""))
        for bounty_id in created.intersection(settlements):
            occurred_at = settlements[bounty_id]
            completed_posts[(source, bounty_id)] = {
                "title": bounty_label(bounty_id),
                "status": f"Settled {occurred_at[:10]}" if occurred_at else "Settled",
                "occurred_at": occurred_at,
            }

    weekly = leaderboard.get("weekly")
    ranking = weekly.get("ranking") if isinstance(weekly, dict) else None
    entries = ranking.get("entries") if isinstance(ranking, dict) else None
    if not isinstance(entries, list):
        raise ValueError("leaderboard_evidence_invalid")
    linked_ranks = [
        entry.get("rank")
        for entry in entries
        if isinstance(entry, dict)
        and str(entry.get("solver_wallet", "")).lower() in addresses
        and isinstance(entry.get("rank"), int)
        and entry["rank"] > 0
    ]

    participating_items = sorted(
        participating.values(), key=lambda item: item["occurred_at"], reverse=True
    )
    completed_items = sorted(
        completed_posts.values(), key=lambda item: item["occurred_at"], reverse=True
    )
    for items in (participating_items, completed_items):
        for item in items:
            item.pop("occurred_at", None)

    return {
        "schema_version": "agent-bounties/local-account-dashboard-v1",
        "data_status": "available",
        "reason": None,
        "identity_link_status": "verified",
        "wallets": wallets,
        "stats": {
            "participating_bounties": len(participating),
            "completed_posted_bounties": len(completed_posts),
            "earned_usdc": usdc_from_base_units(earned_base_units),
            "spent_usdc": usdc_from_base_units(spent_base_units),
            "leaderboard_rank": min(linked_ranks) if linked_ranks else None,
        },
        "activities": {
            "participating": participating_items[:6],
            "completed_posts": completed_items[:6],
        },
        "evidence_boundary": (
            "Wallet ownership was verified with a one-time EIP-191 signature. Earnings count canonical solver rewards and timeout bonuses; "
            "spending counts gross canonical FundingAdded contributions. Rank is the best current weekly rank among linked wallets."
        ),
    }


def exchange_code(provider: str, config: dict[str, str], code: str) -> dict[str, str]:
    token = request_json(
        config["token_url"],
        method="POST",
        data={
            "client_id": config["client_id"],
            "client_secret": config["client_secret"],
            "code": code,
            "redirect_uri": config["redirect_uri"],
            "grant_type": "authorization_code",
        },
    )
    if not isinstance(token, dict) or not token.get("access_token"):
        raise ValueError("provider did not return an access token")
    bearer = {"Authorization": f"Bearer {token['access_token']}"}
    profile = request_json(config["user_url"], headers=bearer)
    if not isinstance(profile, dict):
        raise ValueError("provider returned an invalid profile")

    if provider == "google":
        if not profile.get("sub"):
            raise ValueError("Google profile is missing a subject")
        if profile.get("email") and profile.get("email_verified") is not True:
            raise ValueError("Google profile email is not verified")
        return {
            "provider": "google",
            "sub": str(profile["sub"]),
            "name": str(profile.get("name") or profile.get("email") or "Google user"),
            "email": str(profile.get("email") or ""),
            "avatar": str(profile.get("picture") or ""),
        }

    if provider == "github":
        email = str(profile.get("email") or "")
        if not email:
            emails = request_json("https://api.github.com/user/emails", headers=bearer)
            if isinstance(emails, list):
                verified = [entry for entry in emails if isinstance(entry, dict) and entry.get("verified")]
                primary = next((entry for entry in verified if entry.get("primary")), None)
                chosen = primary or (verified[0] if verified else None)
                email = str((chosen or {}).get("email") or "")
        if not profile.get("id"):
            raise ValueError("GitHub profile is missing an id")
        return {
            "provider": "github",
            "sub": str(profile["id"]),
            "name": str(profile.get("name") or profile.get("login") or "GitHub user"),
            "email": email,
            "avatar": str(profile.get("avatar_url") or ""),
        }

    if provider == "microsoft":
        if not profile.get("sub"):
            raise ValueError("Microsoft profile is missing a subject")
        email = str(profile.get("email") or profile.get("preferred_username") or "")
        return {
            "provider": "microsoft",
            "sub": str(profile["sub"]),
            "name": str(profile.get("name") or email or "Microsoft user"),
            "email": email,
            "avatar": "",
        }

    if provider == "amazon":
        if not profile.get("user_id"):
            raise ValueError("Amazon profile is missing a user id")
        return {
            "provider": "amazon",
            "sub": str(profile["user_id"]),
            "name": str(profile.get("name") or profile.get("email") or "Amazon user"),
            "email": str(profile.get("email") or ""),
            "avatar": "",
        }

    raise ValueError("unsupported provider")


def purge_states(now: float | None = None) -> None:
    current = time.time() if now is None else now
    with STATE_LOCK:
        stale = [key for key, value in OAUTH_STATES.items() if current - float(value["created_at"]) > STATE_MAX_AGE]
        for key in stale:
            OAUTH_STATES.pop(key, None)


def purge_wallet_challenges(now: float | None = None) -> None:
    current = time.time() if now is None else now
    with WALLET_CHALLENGE_LOCK:
        stale = [
            key
            for key, value in WALLET_CHALLENGES.items()
            if current >= float(value.get("expires_at", 0))
        ]
        for key in stale:
            WALLET_CHALLENGES.pop(key, None)


def normalize_email(value: Any) -> str:
    email = unicodedata.normalize("NFC", str(value or "")).strip().lower()
    if len(email) > 320 or any(character.isspace() for character in email):
        raise ValueError("email_invalid")
    if email.count("@") != 1:
        raise ValueError("email_invalid")
    local, domain = email.split("@", 1)
    if not local or len(local) > 64 or "." not in domain or domain.startswith(".") or domain.endswith("."):
        raise ValueError("email_invalid")
    return email


def normalize_password(value: Any) -> str:
    password = unicodedata.normalize("NFC", str(value or ""))
    if not 15 <= len(password) <= 128:
        raise ValueError("password_length_invalid")
    if password.strip().lower() in {
        "passwordpassword",
        "password123456",
        "123456789012345",
        "qwertyuiopasdfgh",
        "letmeinletmein",
        "correcthorsebatterystaple",
        "correct horse battery staple",
        "agentbounties",
        "agentbounties.app",
    }:
        raise ValueError("password_common")
    return password


def local_password_hash(password: str, salt: bytes | None = None) -> str:
    salt = secrets.token_bytes(16) if salt is None else salt
    digest = hashlib.scrypt(password.encode("utf-8"), salt=salt, n=2**14, r=8, p=1, dklen=32)
    return f"{salt.hex()}:{digest.hex()}"


def local_password_verify(stored: str, password: str) -> bool:
    try:
        salt_hex, expected = stored.split(":", 1)
        candidate = local_password_hash(password, bytes.fromhex(salt_hex)).split(":", 1)[1]
        return hmac.compare_digest(candidate, expected)
    except (ValueError, TypeError):
        return False


class LocalAuthHandler(SimpleHTTPRequestHandler):
    server_version = "AgentBountiesLocalAuth/1.0"

    @property
    def auth_env(self) -> dict[str, str]:
        return self.server.auth_env  # type: ignore[attr-defined]

    @property
    def session_secret(self) -> str:
        return self.auth_env["AUTH_SESSION_SECRET"]

    @property
    def wallet_store(self) -> WalletLinkStore:
        return self.server.wallet_store  # type: ignore[attr-defined]

    @property
    def api_base_url(self) -> str:
        return self.server.api_base_url  # type: ignore[attr-defined]

    def end_headers(self) -> None:
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "strict-origin-when-cross-origin")
        self.send_header("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
        super().end_headers()

    def log_message(self, fmt: str, *args: Any) -> None:
        sys.stderr.write(f"[{self.log_date_time_string()}] {fmt % args}\n")

    def send_json(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def redirect(self, location: str, *, cookie: str | None = None) -> None:
        self.send_response(HTTPStatus.FOUND)
        self.send_header("Location", location)
        self.send_header("Cache-Control", "no-store")
        if cookie:
            self.send_header("Set-Cookie", cookie)
        self.end_headers()

    def current_user(self) -> dict[str, Any] | None:
        cookie = SimpleCookie(self.headers.get("Cookie", ""))
        morsel = cookie.get(SESSION_COOKIE)
        user = verify_session(morsel.value, self.session_secret) if morsel else None
        if user and user.get("provider") == "password":
            with self.server.password_lock:  # type: ignore[attr-defined]
                credential = self.server.password_credentials.get(str(user.get("email", "")).lower())  # type: ignore[attr-defined]
            if not credential or int(user.get("credential_version", 0)) != int(credential["version"]):
                return None
        return user

    def read_json_body(self, maximum_bytes: int = 8_192) -> dict[str, Any]:
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError as error:
            raise ValueError("invalid_content_length") from error
        if length <= 0 or length > maximum_bytes:
            raise ValueError("invalid_request_body")
        try:
            payload = json.loads(self.rfile.read(length).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError("invalid_request_body") from error
        if not isinstance(payload, dict):
            raise ValueError("invalid_request_body")
        return payload

    def browser_origin(self) -> str | None:
        origin = self.headers.get("Origin", "")
        try:
            parsed = urllib.parse.urlsplit(origin)
            port = parsed.port or (80 if parsed.scheme == "http" else 443)
        except ValueError:
            return None
        expected_port = int(self.server.server_address[1])  # type: ignore[attr-defined]
        if parsed.scheme != "http" or parsed.hostname not in LOOPBACK_HOSTS or port != expected_port:
            return None
        return f"http://{parsed.hostname}:{port}"

    def do_GET(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlsplit(self.path)
        if parsed.path == "/healthz":
            self.send_json(HTTPStatus.OK, {"ok": True, "providers": configured_providers(self.auth_env), "password": True, "email_delivery": "capture", "storage": "local-preview"})
            return
        if parsed.path == "/auth/dev/captured-mail":
            with self.server.password_lock:  # type: ignore[attr-defined]
                messages = list(self.server.captured_mail)  # type: ignore[attr-defined]
            self.send_json(HTTPStatus.OK, {"messages": messages})
            return
        if parsed.path == "/auth/session":
            user = self.current_user()
            self.send_json(
                HTTPStatus.OK,
                {
                    "authenticated": bool(user),
                    "user": ({key: user.get(key, "") for key in ("name", "email", "avatar")} if user else None),
                    "sign_in_method": user.get("provider") if user else None,
                    "linked_methods": [user.get("provider")] if user else [],
                    "providers": configured_providers(self.auth_env),
                    "password": True,
                },
            )
            return
        if parsed.path == "/auth/account":
            user = self.current_user()
            if not user:
                self.send_json(HTTPStatus.UNAUTHORIZED, {"error": "authentication_required"})
                return
            try:
                wallets = self.wallet_store.wallets_for(user)
            except (OSError, ValueError, json.JSONDecodeError):
                self.send_json(
                    HTTPStatus.OK,
                    unavailable_account_dashboard("wallet_link_store_unavailable"),
                )
                return
            if not wallets:
                self.send_json(
                    HTTPStatus.OK,
                    unavailable_account_dashboard("marketplace_identity_unlinked"),
                )
                return
            try:
                evidence = public_account_evidence(self.api_base_url)
                dashboard = build_linked_account_dashboard(wallets, evidence)
            except (ValueError, TypeError, urllib.error.URLError, urllib.error.HTTPError, TimeoutError):
                dashboard = unavailable_account_dashboard("marketplace_evidence_unavailable", wallets)
            self.send_json(HTTPStatus.OK, dashboard)
            return
        if parsed.path.startswith("/auth/login/"):
            self.begin_oauth(parsed.path.rsplit("/", 1)[-1])
            return
        if parsed.path.startswith("/auth/callback/"):
            self.finish_oauth(parsed.path.rsplit("/", 1)[-1], urllib.parse.parse_qs(parsed.query))
            return
        super().do_GET()

    def do_POST(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlsplit(self.path)
        if parsed.path == "/auth/logout":
            self.send_response(HTTPStatus.NO_CONTENT)
            self.send_header(
                "Set-Cookie",
                f"{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
            )
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            return
        if parsed.path == "/auth/password/registration":
            self.begin_password_action("registration")
            return
        if parsed.path == "/auth/password/reset":
            self.begin_password_action("reset")
            return
        if parsed.path == "/auth/password/verification":
            self.verify_password_action("registration")
            return
        if parsed.path == "/auth/password/reset-verification":
            self.verify_password_action("reset")
            return
        if parsed.path == "/auth/password/complete":
            self.complete_password_action("registration")
            return
        if parsed.path == "/auth/password/reset-complete":
            self.complete_password_action("reset")
            return
        if parsed.path == "/auth/password/login":
            self.password_login()
            return
        if parsed.path == "/auth/wallet/challenge":
            self.begin_wallet_link()
            return
        if parsed.path == "/auth/wallet/verify":
            self.finish_wallet_link()
            return
        if parsed.path == "/auth/wallet/unlink":
            self.unlink_wallet()
            return
        self.send_json(HTTPStatus.NOT_FOUND, {"error": "not_found"})

    def password_request(self) -> dict[str, Any] | None:
        if not self.browser_origin():
            self.send_json(HTTPStatus.FORBIDDEN, {"error": "invalid_origin"})
            return None
        try:
            return self.read_json_body()
        except ValueError as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
            return None

    def begin_password_action(self, purpose: str) -> None:
        payload = self.password_request()
        if payload is None:
            return
        try:
            display_email = unicodedata.normalize("NFC", str(payload.get("email") or "")).strip()
            email_key = normalize_email(display_email)
        except ValueError as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
            return
        with self.server.password_lock:  # type: ignore[attr-defined]
            credential = self.server.password_credentials.get(email_key)  # type: ignore[attr-defined]
            should_send = (purpose == "registration" and credential is None) or (purpose == "reset" and credential is not None)
            if should_send:
                token = secrets.token_hex(32)
                token_hash = hashlib.sha256(token.encode("ascii")).hexdigest()
                lifetime = SESSION_MAX_AGE if purpose == "registration" else 30 * 60
                self.server.password_actions[token_hash] = {  # type: ignore[attr-defined]
                    "purpose": purpose,
                    "email": display_email,
                    "email_key": email_key,
                    "expires_at": time.time() + lifetime,
                }
                action = "register" if purpose == "registration" else "reset"
                origin = self.browser_origin() or f"http://{self.server.server_address[0]}:{self.server.server_address[1]}"  # type: ignore[attr-defined]
                self.server.captured_mail.append({  # type: ignore[attr-defined]
                    "to": display_email,
                    "purpose": purpose,
                    "action_url": f"{origin}/#auth={action}&token={token}",
                    "created_at": utc_iso(time.time()),
                })
        self.send_json(
            HTTPStatus.ACCEPTED,
            {"accepted": True, "message": "If this address can continue, an email with the next step will arrive shortly."},
        )

    def verify_password_action(self, purpose: str) -> None:
        payload = self.password_request()
        if payload is None:
            return
        token = str(payload.get("token") or "")
        token_hash = hashlib.sha256(token.encode("ascii", errors="ignore")).hexdigest()
        with self.server.password_lock:  # type: ignore[attr-defined]
            action = self.server.password_actions.pop(token_hash, None)  # type: ignore[attr-defined]
            if not action or action["purpose"] != purpose or float(action["expires_at"]) <= time.time():
                self.send_json(HTTPStatus.BAD_REQUEST, {"error": "email_action_invalid"})
                return
            setup = secrets.token_hex(32)
            setup_hash = hashlib.sha256(setup.encode("ascii")).hexdigest()
            self.server.password_setups[setup_hash] = action  # type: ignore[attr-defined]
        self.send_response(HTTPStatus.OK)
        body = json.dumps({"verified": True, "purpose": purpose, "email": action["email"]}, separators=(",", ":")).encode("utf-8")
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Set-Cookie", f"{PASSWORD_SETUP_COOKIE}={setup}; Path=/auth/password; HttpOnly; SameSite=Strict; Max-Age=1800")
        self.end_headers()
        self.wfile.write(body)

    def complete_password_action(self, purpose: str) -> None:
        payload = self.password_request()
        if payload is None:
            return
        cookie = SimpleCookie(self.headers.get("Cookie", ""))
        morsel = cookie.get(PASSWORD_SETUP_COOKIE)
        setup_hash = hashlib.sha256((morsel.value if morsel else "").encode("ascii")).hexdigest()
        try:
            password = normalize_password(payload.get("password"))
            name = unicodedata.normalize("NFC", str(payload.get("name") or "")).strip()
            if purpose == "registration" and not 1 <= len(name) <= 160:
                raise ValueError("name_invalid")
        except ValueError as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
            return
        with self.server.password_lock:  # type: ignore[attr-defined]
            action = self.server.password_setups.pop(setup_hash, None)  # type: ignore[attr-defined]
            if not action or action["purpose"] != purpose or float(action["expires_at"]) <= time.time():
                self.send_json(HTTPStatus.BAD_REQUEST, {"error": "email_action_invalid"})
                return
            prior = self.server.password_credentials.get(action["email_key"])  # type: ignore[attr-defined]
            version = int(prior.get("version", 0) if prior else 0) + 1
            if purpose == "reset" and prior:
                name = str(prior["name"])
            self.server.password_credentials[action["email_key"]] = {  # type: ignore[attr-defined]
                "email": action["email"],
                "name": name,
                "password_hash": local_password_hash(password),
                "version": version,
            }
        self.password_session(action["email_key"], self.server.password_credentials[action["email_key"]])  # type: ignore[attr-defined]

    def password_login(self) -> None:
        payload = self.password_request()
        if payload is None:
            return
        try:
            email_key = normalize_email(payload.get("email"))
            password = unicodedata.normalize("NFC", str(payload.get("password") or ""))
        except ValueError:
            email_key = "invalid"
            password = str(payload.get("password") or "")
        with self.server.password_lock:  # type: ignore[attr-defined]
            credential = self.server.password_credentials.get(email_key)  # type: ignore[attr-defined]
        dummy = "00" * 16 + ":" + "00" * 32
        valid = local_password_verify(str(credential["password_hash"]) if credential else dummy, password)
        if not credential or not valid:
            self.send_json(HTTPStatus.UNAUTHORIZED, {"error": "invalid_credentials"})
            return
        self.password_session(email_key, credential)

    def password_session(self, email_key: str, credential: dict[str, Any]) -> None:
        user = {
            "provider": "password",
            "sub": email_key,
            "name": credential["name"],
            "email": credential["email"],
            "avatar": "",
            "credential_version": credential["version"],
        }
        session = sign_session(user, self.session_secret)
        body = b'{"authenticated":true}'
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Set-Cookie", f"{SESSION_COOKIE}={session}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_MAX_AGE}")
        self.send_header("Set-Cookie", f"{PASSWORD_SETUP_COOKIE}=; Path=/auth/password; HttpOnly; SameSite=Strict; Max-Age=0")
        self.end_headers()
        self.wfile.write(body)

    def wallet_request_context(self) -> tuple[dict[str, Any], str, dict[str, Any]] | None:
        user = self.current_user()
        if not user:
            self.send_json(HTTPStatus.UNAUTHORIZED, {"error": "authentication_required"})
            return None
        origin = self.browser_origin()
        if not origin:
            self.send_json(HTTPStatus.FORBIDDEN, {"error": "invalid_origin"})
            return None
        try:
            payload = self.read_json_body()
        except ValueError as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
            return None
        return user, origin, payload

    def begin_wallet_link(self) -> None:
        context = self.wallet_request_context()
        if not context:
            return
        user, origin, payload = context
        try:
            address = normalize_wallet_address(payload.get("address"))
        except ValueError as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
            return
        purge_wallet_challenges()
        now = int(time.time())
        expires_at = now + WALLET_CHALLENGE_MAX_AGE
        challenge_id = secrets.token_urlsafe(24)
        nonce = secrets.token_urlsafe(18)
        account_id = self.wallet_store.account_id(user)
        message = wallet_link_message(origin, account_id, address, nonce, now, expires_at)
        with WALLET_CHALLENGE_LOCK:
            WALLET_CHALLENGES[challenge_id] = {
                "account_id": account_id,
                "address": address,
                "message": message,
                "expires_at": expires_at,
            }
        self.send_json(
            HTTPStatus.CREATED,
            {
                "challenge_id": challenge_id,
                "address": address,
                "chain_id": BASE_CHAIN_ID,
                "message": message,
                "expires_at": utc_iso(expires_at),
                "intent": "Prove wallet ownership only; no transaction, approval, or payment.",
            },
        )

    def finish_wallet_link(self) -> None:
        context = self.wallet_request_context()
        if not context:
            return
        user, _origin, payload = context
        challenge_id = str(payload.get("challenge_id") or "")
        try:
            address = normalize_wallet_address(payload.get("address"))
        except ValueError as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
            return
        with WALLET_CHALLENGE_LOCK:
            challenge = WALLET_CHALLENGES.pop(challenge_id, None)
        if (
            not challenge
            or time.time() >= float(challenge.get("expires_at", 0))
            or challenge.get("account_id") != self.wallet_store.account_id(user)
            or challenge.get("address") != address
        ):
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": "wallet_challenge_invalid"})
            return
        if Account is None or encode_defunct is None:
            self.send_json(HTTPStatus.SERVICE_UNAVAILABLE, {"error": "signature_verifier_unavailable"})
            return
        if not verify_wallet_signature(str(challenge["message"]), payload.get("signature"), address):
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": "wallet_signature_invalid"})
            return
        try:
            wallets = self.wallet_store.link(user, address, utc_iso(time.time()))
        except ValueError as error:
            status = HTTPStatus.CONFLICT if str(error) in {
                "wallet_linked_to_another_account",
                "wallet_limit_reached",
            } else HTTPStatus.INTERNAL_SERVER_ERROR
            self.send_json(status, {"error": str(error)})
            return
        except OSError:
            self.send_json(HTTPStatus.INTERNAL_SERVER_ERROR, {"error": "wallet_link_store_unavailable"})
            return
        self.send_json(HTTPStatus.OK, {"linked": True, "wallets": wallets})

    def unlink_wallet(self) -> None:
        context = self.wallet_request_context()
        if not context:
            return
        user, _origin, payload = context
        try:
            wallets = self.wallet_store.unlink(user, normalize_wallet_address(payload.get("address")))
        except ValueError as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
            return
        except OSError:
            self.send_json(HTTPStatus.INTERNAL_SERVER_ERROR, {"error": "wallet_link_store_unavailable"})
            return
        self.send_json(HTTPStatus.OK, {"unlinked": True, "wallets": wallets})

    def begin_oauth(self, provider: str) -> None:
        config = provider_config(self.auth_env, provider)
        if not config or not config["client_id"] or not config["client_secret"]:
            self.redirect(f"/?auth=error&reason={urllib.parse.quote(provider + '_not_configured')}")
            return
        purge_states()
        state = secrets.token_urlsafe(32)
        with STATE_LOCK:
            OAUTH_STATES[state] = {"provider": provider, "created_at": time.time()}
        self.redirect(authorization_url(provider, config, state))

    def finish_oauth(self, provider: str, query: dict[str, list[str]]) -> None:
        state = (query.get("state") or [""])[0]
        code = (query.get("code") or [""])[0]
        provider_error = (query.get("error") or [""])[0]
        with STATE_LOCK:
            state_record = OAUTH_STATES.pop(state, None)
        if provider_error:
            self.redirect("/?auth=error&reason=access_denied")
            return
        if not state_record or state_record.get("provider") != provider or not code:
            self.redirect("/?auth=error&reason=invalid_state")
            return
        if time.time() - float(state_record["created_at"]) > STATE_MAX_AGE:
            self.redirect("/?auth=error&reason=expired_state")
            return
        config = provider_config(self.auth_env, provider)
        if not config:
            self.redirect("/?auth=error&reason=provider_not_configured")
            return
        try:
            user = exchange_code(provider, config, code)
        except (ValueError, urllib.error.URLError, urllib.error.HTTPError, TimeoutError):
            self.redirect("/?auth=error&reason=provider_exchange_failed")
            return
        session = sign_session(user, self.session_secret)
        cookie = f"{SESSION_COOKIE}={session}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_MAX_AGE}"
        self.redirect(f"/?auth=success&provider={urllib.parse.quote(provider)}", cookie=cookie)


class LocalAuthServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(
        self,
        address: tuple[str, int],
        handler: type[LocalAuthHandler],
        env: dict[str, str],
        wallet_links_path: Path = DEFAULT_WALLET_LINKS,
    ):
        self.auth_env = env
        wallet_secret = env.get("AUTH_WALLET_LINK_SECRET") or env["AUTH_SESSION_SECRET"]
        self.wallet_store = WalletLinkStore(wallet_links_path.resolve(), wallet_secret)
        self.api_base_url = env.get("AGENT_BOUNTIES_API_BASE_URL", "https://api.agentbounties.app").rstrip("/")
        self.password_credentials: dict[str, dict[str, Any]] = {}
        self.password_actions: dict[str, dict[str, Any]] = {}
        self.password_setups: dict[str, dict[str, Any]] = {}
        self.captured_mail: list[dict[str, Any]] = []
        self.password_lock = threading.Lock()
        super().__init__(address, handler)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=4173)
    parser.add_argument("--site", type=Path, default=DEFAULT_SITE)
    parser.add_argument("--env-file", type=Path, default=DEFAULT_ENV)
    parser.add_argument("--wallet-links-file", type=Path, default=DEFAULT_WALLET_LINKS)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.host not in LOOPBACK_HOSTS:
        raise SystemExit("Refusing to expose the local OAuth server on a non-loopback host.")
    site = args.site.resolve()
    if not site.is_dir():
        raise SystemExit(f"Site directory does not exist: {site}")
    env = {**os.environ, **load_env_file(args.env_file.resolve())}
    if len(env.get("AUTH_SESSION_SECRET", "")) < 32:
        raise SystemExit("AUTH_SESSION_SECRET must be at least 32 characters in .env.auth.local")

    def handler(*handler_args: Any, **handler_kwargs: Any) -> LocalAuthHandler:
        return LocalAuthHandler(*handler_args, directory=str(site), **handler_kwargs)

    server = LocalAuthServer(
        (args.host, args.port),
        handler,
        env,
        args.wallet_links_file,
    )  # type: ignore[arg-type]
    providers = ", ".join(name for name, ready in configured_providers(env).items() if ready) or "none"
    print(f"Agent Bounties local auth: http://{args.host}:{args.port}/ (providers: {providers})", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
