import hashlib
import os
import secrets
import time
import uuid
from typing import Callable

import httpx
from eth_hash.auto import keccak


OPEN_COMPETITION_COMMITMENT_SCHEMA = (
    "agent-bounties/open-competition-v1-commitment-v1"
)
OPEN_COMPETITION_V2_JOURNAL_DOMAIN = bytes.fromhex(
    "110d7acc5c3397f452c974ba4f7296d7d2a2cede57290113d1fd256e1818804b"
)
OPEN_COMPETITION_V2_POLICY_DOMAIN = bytes.fromhex(
    "f6a226ca20aaca3b9c0b4a609939c334b6c2b03500a5df45188df8bcd7c2b369"
)
OPEN_COMPETITION_V2_SUBMISSION_DOMAIN = bytes.fromhex(
    "402204460b00978c26cee42ae0089d94fe8b0b17bd90c45a6cd78d466463a507"
)
OPEN_COMPETITION_V2_EVIDENCE_DOMAIN = bytes.fromhex(
    "16f60f26d350a38e6993a5454967d1efb0461d93785b7cdb38ba463284c5ab15"
)
OPEN_COMPETITION_V2_JOURNAL_SCHEMA_HASH = bytes.fromhex(
    "d9c492538aa0822e8a1d651886e79a2b8ddfc2c3428b3ed92e19d337eefe77d4"
)
OPEN_COMPETITION_V2_METRIC_PROGRAM_HASH = bytes.fromhex(
    "1c27fc20ab65264c7db2997c8b76f78d7291cdb91243481bcae1e88f77beb88a"
)
OPEN_COMPETITION_V2_PROOF_SYSTEMS = {
    "groth16": bytes.fromhex(
        "0fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d"
    ),
    "plonk": bytes.fromhex(
        "91e36d74d5d8703299314b82f85cab384a3df8064725b371f1f9f4ad49238f1b"
    ),
}
OPEN_COMPETITION_V2_MODE_TAGS = {
    "all_equal": 0,
    "maximize_exact_matches": 1,
    "minimize_absolute_error": 2,
}
OPEN_COMPETITION_V2_MAXIMUM_VECTORS = 10_000


def _open_competition_address_word(value: str) -> bytes:
    if not isinstance(value, str) or not value.startswith("0x") or len(value) != 42:
        raise ValueError("expected a 20-byte EVM address")
    try:
        raw = bytes.fromhex(value[2:])
    except ValueError as error:
        raise ValueError("expected a 20-byte EVM address") from error
    return bytes(12) + raw


def _open_competition_bytes32(value: str, label: str) -> bytes:
    if not isinstance(value, str) or not value.startswith("0x") or len(value) != 66:
        raise ValueError(f"{label} must be a 32-byte 0x-prefixed hex value")
    try:
        raw = bytes.fromhex(value[2:])
    except ValueError as error:
        raise ValueError(f"{label} must be a 32-byte 0x-prefixed hex value") from error
    if not any(raw):
        raise ValueError(f"{label} must be nonzero")
    return raw


def generate_open_competition_commitment(
    *,
    network: str,
    bounty: str,
    solver: str,
    submission_hash: str,
    evidence_hash: str,
) -> dict:
    """Generate a local recovery envelope; never send it to commit preparation."""
    chain_ids = {"base-mainnet": 8453, "base-sepolia": 84532}
    if network not in chain_ids:
        raise ValueError("network must be base-mainnet or base-sepolia")
    salt = secrets.token_bytes(32)
    submission = _open_competition_bytes32(submission_hash, "submission_hash")
    evidence = _open_competition_bytes32(evidence_hash, "evidence_hash")
    domain = keccak(b"agent-bounties/open-competition-v1-solution")
    encoded = b"".join(
        [
            domain,
            chain_ids[network].to_bytes(32, "big"),
            _open_competition_address_word(bounty),
            _open_competition_address_word(solver),
            submission,
            evidence,
            salt,
        ]
    )
    return {
        "schema_version": OPEN_COMPETITION_COMMITMENT_SCHEMA,
        "network": network,
        "chain_id": chain_ids[network],
        "bounty": bounty.lower(),
        "solver": solver.lower(),
        "submission_hash": submission_hash.lower(),
        "evidence_hash": evidence_hash.lower(),
        "salt": f"0x{salt.hex()}",
        "commitment": f"0x{keccak(encoded).hex()}",
        "committed_block": None,
        "reveal_deadline": None,
        "evidence_boundary": (
            "This recovery envelope contains the secret salt. Store it locally and "
            "send only commitment during entry preparation."
        ),
    }


def build_open_competition_v2_public_vector(input: dict) -> dict:
    """Build the exact public-vector-metric-v1 input commitments and journal."""
    scope = input.get("scope")
    vectors = input.get("vectors")
    mode = input.get("mode")
    threshold = input.get("threshold")
    if not isinstance(scope, dict):
        raise ValueError("scope must be an object")
    if mode not in OPEN_COMPETITION_V2_MODE_TAGS:
        raise ValueError("mode is unsupported")
    if not isinstance(threshold, int) or isinstance(threshold, bool) or not -(2**127) <= threshold < 2**127:
        raise ValueError("threshold must be an i128 integer")
    if mode != "all_equal" and threshold < 0:
        raise ValueError("threshold cannot be negative for this mode")
    if not isinstance(vectors, list) or not 1 <= len(vectors) <= OPEN_COMPETITION_V2_MAXIMUM_VECTORS:
        raise ValueError("vectors must contain 1..10000 cases")

    score = 0
    total_weight = 0
    policy = bytearray(OPEN_COMPETITION_V2_POLICY_DOMAIN)
    policy.append(OPEN_COMPETITION_V2_MODE_TAGS[mode])
    policy.extend(threshold.to_bytes(16, "big", signed=True))
    policy.extend(len(vectors).to_bytes(4, "big"))
    submission = bytearray(OPEN_COMPETITION_V2_SUBMISSION_DOMAIN)
    submission.extend(len(vectors).to_bytes(4, "big"))
    for vector in vectors:
        if not isinstance(vector, dict):
            raise ValueError("each vector must be an object")
        expected = vector.get("expected")
        observed = vector.get("observed")
        weight = vector.get("weight")
        if any(isinstance(value, bool) or not isinstance(value, int) for value in (expected, observed, weight)):
            raise ValueError("expected, observed, and weight must be integers")
        if not -(2**63) <= expected < 2**63 or not -(2**63) <= observed < 2**63:
            raise ValueError("expected and observed must fit i64")
        if not 1 <= weight < 2**32:
            raise ValueError("weight must fit positive u32")
        total_weight += weight
        if mode in ("all_equal", "maximize_exact_matches"):
            if expected == observed:
                score += weight
        else:
            score += abs(expected - observed) * weight
        if not -(2**127) <= score < 2**127 or total_weight >= 2**127:
            raise ValueError("metric arithmetic overflow")
        policy.extend(expected.to_bytes(8, "big", signed=True))
        policy.extend(weight.to_bytes(4, "big"))
        submission.extend(observed.to_bytes(8, "big", signed=True))

    passed = (
        score == total_weight
        if mode == "all_equal"
        else score >= threshold
        if mode == "maximize_exact_matches"
        else score <= threshold
    )
    verification_policy_hash = keccak(bytes(policy))
    submission_hash = keccak(bytes(submission))
    evidence_hash = keccak(
        OPEN_COMPETITION_V2_EVIDENCE_DOMAIN
        + verification_policy_hash
        + submission_hash
    )
    proof_system = scope.get("proof_system")
    if proof_system not in OPEN_COMPETITION_V2_PROOF_SYSTEMS:
        raise ValueError("scope.proof_system must be groth16 or plonk")
    words = [
        OPEN_COMPETITION_V2_JOURNAL_DOMAIN,
        _v2_uint_word(scope.get("chain_id"), "scope.chain_id", 64),
        _open_competition_address_word(scope.get("competition")),
        _open_competition_bytes32(scope.get("bounty_id"), "scope.bounty_id"),
        _open_competition_address_word(scope.get("solver")),
        _v2_uint_word(scope.get("solver_nonce"), "scope.solver_nonce", 128),
        submission_hash,
        evidence_hash,
        OPEN_COMPETITION_V2_PROOF_SYSTEMS[proof_system],
        _open_competition_bytes32(scope.get("program_vkey"), "scope.program_vkey"),
        _open_competition_bytes32(scope.get("source_hash"), "scope.source_hash"),
        _open_competition_bytes32(scope.get("elf_hash"), "scope.elf_hash"),
        OPEN_COMPETITION_V2_JOURNAL_SCHEMA_HASH,
        OPEN_COMPETITION_V2_METRIC_PROGRAM_HASH,
        _open_competition_bytes32(scope.get("execution_policy_hash"), "scope.execution_policy_hash"),
        verification_policy_hash,
        _open_competition_bytes32(scope.get("settlement_policy_hash"), "scope.settlement_policy_hash"),
        _open_competition_bytes32(scope.get("beta_risk_hash"), "scope.beta_risk_hash"),
        _v2_uint_word(1 if passed else 0, "passed", 8),
        score.to_bytes(32, "big", signed=True),
    ]
    journal = b"".join(words)
    return {
        "passed": passed,
        "score": str(score),
        "verification_policy_hash": f"0x{verification_policy_hash.hex()}",
        "submission_hash": f"0x{submission_hash.hex()}",
        "evidence_hash": f"0x{evidence_hash.hex()}",
        "journal_hex": f"0x{journal.hex()}",
    }


def _v2_uint_word(value, label: str, bits: int) -> bytes:
    if not isinstance(value, int) or isinstance(value, bool) or not 0 <= value < 2**bits:
        raise ValueError(f"{label} must fit u{bits}")
    return value.to_bytes(32, "big")


class AgentBountiesHttpError(httpx.HTTPStatusError):
    """HTTP error that preserves the platform's parsed machine-readable problem."""

    def __init__(self, response: httpx.Response, body):
        super().__init__(
            f"{response.request.url.path} failed: {response.status_code}",
            request=response.request,
            response=response,
        )
        self.status_code = response.status_code
        self.body = body


def hash_artifact(body: str) -> str:
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


def _x402_response_body(response: httpx.Response) -> dict:
    try:
        body = response.json()
    except ValueError:
        return {"error": response.text or f"HTTP {response.status_code}"}
    return body if isinstance(body, dict) else {"error": response.text}


class AgentBountiesClient:
    def __init__(
        self,
        base_url: str = "http://127.0.0.1:8080",
        operator_api_token: str | None = None,
        analytics_exclusion_token: str | None = None,
    ):
        self.base_url = base_url.rstrip("/")
        self.operator_api_token = operator_api_token or os.getenv("OPERATOR_API_TOKEN")
        self.analytics_exclusion_token = analytics_exclusion_token or os.getenv(
            "AGENT_BOUNTIES_ANALYTICS_EXCLUSION_TOKEN"
        )

    def _headers(self) -> dict[str, str]:
        headers = {"x-agent-bounties-interface": "api"}
        if self.operator_api_token:
            headers["x-operator-token"] = self.operator_api_token
        if self.analytics_exclusion_token:
            headers["x-agent-bounties-analytics-exclusion"] = (
                self.analytics_exclusion_token
            )
        return headers

    def _http(
        self,
        method: str,
        path: str,
        *,
        json: dict | None = None,
        params: dict | None = None,
        headers: dict[str, str] | None = None,
    ) -> httpx.Response:
        request_headers = self._headers() or {}
        request_headers.update(headers or {})
        return httpx.request(
            method,
            f"{self.base_url}{path}",
            json=json,
            params={key: value for key, value in params.items() if value is not None} if params else None,
            headers=request_headers or None,
            timeout=30,
        )

    def _request(
        self,
        method: str,
        path: str,
        json: dict | None = None,
        params: dict | None = None,
        headers: dict[str, str] | None = None,
    ):
        response = self._http(method, path, json=json, params=params, headers=headers)
        if response.status_code == 204:
            return {"http_status": 204, "success": True}
        try:
            body = response.json()
        except ValueError:
            body = {"error": response.text or f"HTTP {response.status_code}"}
        if response.is_error:
            raise AgentBountiesHttpError(response, body)
        return body

    def _x402_request(
        self,
        path: str,
        accepted_statuses: tuple[int, ...],
        *,
        params: dict | None = None,
        headers: dict[str, str] | None = None,
    ) -> dict:
        response = self._http("GET", path, params=params, headers=headers)
        if response.status_code not in accepted_statuses:
            response.raise_for_status()
        return {
            "status": response.status_code,
            "payment_required": response.headers.get("PAYMENT-REQUIRED"),
            "payment_response": response.headers.get("PAYMENT-RESPONSE"),
            "body": _x402_response_body(response),
        }

    def _stripe_event(self, path: str, event: dict, signature: str | None) -> dict:
        headers = {"stripe-signature": signature} if signature else None
        response = self._http("POST", path, json=event, headers=headers)
        response.raise_for_status()
        return response.json()

    def route_blocked_goal(self, goal: str, context: str, budget_minor: int, currency: str = "usdc", privacy: str = "Public"):
        return self._request(
            "POST",
            "/v1/route-blocked-goal",
            json={
                "goal": goal,
                "context": context,
                "budget_minor": budget_minor,
                "currency": currency,
                "privacy": privacy,
            },
        )

    def get_discovery_manifest(self):
        return self._request("GET", "/.well-known/agent-bounties.json")

    def get_discovery_manifest_schema(self):
        return self._request("GET", "/schemas/discovery-manifest.v2.json")

    def get_x402_discovery(self):
        return self._request("GET", "/.well-known/x402.json")

    def compile_objective(
        self,
        objective: str,
        *,
        context: str | None = None,
        constraints: list[str] | None = None,
        max_tasks: int = 5,
        solver_budget_usdc: str | None = None,
        source_url: str | None = None,
        idempotency_key: str | None = None,
    ):
        """Compile one objective into an advisory, validated bounty graph."""
        return self._request(
            "POST",
            "/v1/cloud-agent/objective-plans",
            json={
                "objective": objective,
                "context": context,
                "constraints": constraints or [],
                "max_tasks": max_tasks,
                "solver_budget_usdc": solver_budget_usdc,
                "source_url": source_url,
                "idempotency_key": idempotency_key,
            },
        )

    def request_x402_bounty_funding(
        self,
        bounty_contract: str,
        amount: int | None = None,
        network: str = "base-mainnet",
        relayer: str | None = None,
        payment_signature: str | None = None,
    ):
        return self._x402_request(
            f"/v1/x402/base/bounties/{bounty_contract}/funding",
            (200, 202, 400, 402, 404, 409, 413, 422, 429, 503),
            params={
                "network": network,
                "amount": amount,
                "relayer": relayer,
            },
            headers={"PAYMENT-SIGNATURE": payment_signature} if payment_signature else None,
        )

    def get_x402_relay_status(self, relay_id: str):
        return self._x402_request(
            f"/v1/x402/base/relays/{relay_id}", (200, 202, 404, 422, 503)
        )

    def fund_x402_bounty(
        self,
        bounty_contract: str,
        signer: Callable[[str, dict], str],
        amount: int | None = None,
        network: str = "base-mainnet",
        relayer: str | None = None,
        poll_interval_seconds: float = 1.0,
        timeout_seconds: float = 60.0,
    ):
        challenge = self.request_x402_bounty_funding(
            bounty_contract,
            amount=amount,
            network=network,
            relayer=relayer,
        )
        payment_required = challenge["payment_required"]
        if challenge["status"] != 402 or not payment_required:
            raise RuntimeError("x402 endpoint did not return a signable PAYMENT-REQUIRED challenge")
        payment_signature = signer(payment_required, challenge["body"])
        if not payment_signature:
            raise RuntimeError("x402 signer returned an empty PAYMENT-SIGNATURE")

        deadline = time.monotonic() + timeout_seconds
        response = self.request_x402_bounty_funding(
            bounty_contract,
            amount=amount,
            network=network,
            relayer=relayer,
            payment_signature=payment_signature,
        )
        while response["status"] != 200:
            if response["status"] == 402:
                raise RuntimeError("x402 authorization expired or no longer matches the challenge")
            if response["status"] == 422:
                raise RuntimeError("x402 authorization failed without canonical funding")
            if response["status"] == 429:
                raise RuntimeError("x402 hosted relay rolling quota is exhausted")
            if response["status"] in (400, 404, 409, 413):
                raise RuntimeError(
                    f"x402 funding request was rejected with HTTP {response['status']}"
                )
            if time.monotonic() >= deadline:
                raise TimeoutError("x402 funding timed out before canonical confirmation")
            time.sleep(poll_interval_seconds)
            relay_id = self._x402_relay_id(response["body"])
            if relay_id:
                response = self.get_x402_relay_status(relay_id)
            else:
                response = self.request_x402_bounty_funding(
                    bounty_contract,
                    amount=amount,
                    network=network,
                    relayer=relayer,
                    payment_signature=payment_signature,
                )
            if response["status"] == 503:
                response = self.request_x402_bounty_funding(
                    bounty_contract,
                    amount=amount,
                    network=network,
                    relayer=relayer,
                    payment_signature=payment_signature,
                )
        if not response["payment_response"]:
            raise RuntimeError("confirmed x402 funding is missing PAYMENT-RESPONSE")
        return response

    @staticmethod
    def _x402_relay_id(body: dict) -> str | None:
        relay = body.get("relay")
        if isinstance(relay, dict) and isinstance(relay.get("id"), str):
            return relay["id"]
        status_url = body.get("statusUrl")
        if isinstance(status_url, str):
            return status_url.rstrip("/").rsplit("/", 1)[-1]
        return None

    def get_risk_policy(self):
        return self._request("GET", "/v1/risk/policy")

    def get_live_money_readiness(self, network: str | None = None):
        return self._request(
            "GET",
            "/v1/readiness/live-money",
            params={"network": network},
        )

    def prepare_agent_to_earn(
        self,
        wallet_address: str,
        bounty_contract: str,
        signing_capabilities: list[str],
        policy: dict,
        network: str = "base-mainnet",
        wallet_profile: str | None = None,
        claim_bond_base_units: str | None = None,
    ):
        """Check public wallet readiness without requesting wallet secrets or a signature."""
        return self._request(
            "POST",
            "/v1/base/agent-wallet/readiness",
            json={
                "network": network,
                "wallet_address": wallet_address,
                "bounty_contract": bounty_contract,
                "claim_bond_base_units": (
                    str(claim_bond_base_units)
                    if claim_bond_base_units is not None
                    else None
                ),
                "signing_capabilities": signing_capabilities,
                "wallet_profile": wallet_profile,
                "policy": policy,
            },
        )

    def get_standing_meta_v4_readiness(self, network: str = "base-mainnet"):
        """Return fail-closed V4 readiness; a report is never payment evidence."""
        return self._request(
            "GET",
            "/v1/base/standing-meta-v4/readiness",
            params={"network": network},
        )

    def list_open_competition_verifiers(self, network: str = "base-mainnet"):
        """List exact catalog-approved deterministic verifier profiles."""
        return self._request(
            "GET",
            "/v1/base/open-competition-v1/verifiers",
            params={"network": network},
        )

    def prepare_open_competition_creation(self, request: dict):
        """Prepare direct or EIP-3009 authorized deterministic competition creation."""
        path = (
            "/v1/base/open-competition-v1/authorized-creation-preparation"
            if request.get("funding_authorization") is not None
            else "/v1/base/open-competition-v1/creation-preparation"
        )
        return self._request("POST", path, json=request)

    def get_open_competition_state(
        self,
        bounty_contract: str,
        network: str = "base-mainnet",
        solver: str | None = None,
        verifier_profile_id: str | None = None,
    ):
        """Return safe-block canonical state; this is not settlement evidence."""
        params = {
            "network": network,
            "bounty_contract": bounty_contract,
            "solver": solver,
            "verifier_profile_id": verifier_profile_id,
        }
        return self._request(
            "GET", "/v1/base/open-competition-v1/state", params=params
        )

    def get_open_competition_readiness(
        self,
        bounty_contract: str,
        network: str = "base-mainnet",
        solver: str | None = None,
        verifier_profile_id: str | None = None,
    ):
        """Return fail-closed first-valid competition readiness."""
        return self._request(
            "GET",
            "/v1/base/open-competition-v1/readiness",
            params={
                "network": network,
                "bounty_contract": bounty_contract,
                "solver": solver,
                "verifier_profile_id": verifier_profile_id,
            },
        )

    def _open_competition_action(
        self,
        path: str,
        bounty_contract: str,
        arguments: dict,
        network: str,
    ):
        return self._request(
            "POST",
            f"/v1/base/open-competition-v1/{path}",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "arguments": arguments,
            },
        )

    def prepare_open_competition_commit(
        self,
        bounty_contract: str,
        solver: str,
        commitment: str,
        network: str = "base-mainnet",
    ):
        """Send only the public commitment; keep the recovery envelope local."""
        return self._request(
            "POST",
            "/v1/base/open-competition-v1/commit-preparation",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "solver": solver,
                "commitment": commitment,
            },
        )

    def prepare_open_competition_reveal(
        self,
        bounty_contract: str,
        solver: str,
        commitment_envelope: dict,
        proof: str,
        network: str = "base-mainnet",
    ):
        """Prepare a reveal only from a locally recovered commitment envelope."""
        return self._request(
            "POST",
            "/v1/base/open-competition-v1/reveal-preparation",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "solver": solver,
                "commitment_envelope": commitment_envelope,
                "proof": proof,
            },
        )

    def get_open_competition_status(
        self, bounty_contract: str, arguments: dict, network: str = "base-mainnet"
    ):
        return self._open_competition_action(
            "status", bounty_contract, arguments, network
        )

    def withdraw_open_competition_bond(
        self, bounty_contract: str, arguments: dict, network: str = "base-mainnet"
    ):
        return self._open_competition_action(
            "bond-withdrawal-preparation", bounty_contract, arguments, network
        )

    def prepare_open_competition_entrant_action(self, request: dict):
        """Prepare one exact policy-bound entrant-wallet EIP-712 action."""
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST",
            "/v1/base/open-competition-v1/entrant-action-preparation",
            json=payload,
        )

    def relay_open_competition_entrant_action(
        self, idempotency_key: str, plan: dict, signature: str
    ):
        """Relay one signed entrant action; only canonical events prove execution."""
        return self._request(
            "POST",
            "/v1/base/open-competition-v1/entrant-action-relays",
            json={
                "idempotency_key": idempotency_key,
                "plan": plan,
                "signature": signature,
            },
        )

    def get_open_competition_entrant_relay(self, relay_id: str):
        """Poll durable relay state through Base safe-block reconciliation."""
        return self._request(
            "GET",
            f"/v1/base/open-competition-v1/entrant-action-relays/{relay_id}",
        )

    def get_open_competition_v2_profiles(self, network: str = "base-mainnet"):
        return self._request(
            "GET",
            "/v1/base/open-competition-v2-beta2/profiles",
            params={"network": network},
        )

    def get_open_competition_v2_release(self, network: str = "base-mainnet"):
        """Return the exact immutable Beta2 identities and activation state."""
        return self._request(
            "GET",
            "/v1/base/open-competition-v2-beta2/release",
            params={"network": network},
        )

    def prepare_open_competition_v2_structured_artifact_profile(self, request: dict):
        """Derive exact reviewed metric fields from deterministic artifact requirements."""
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST",
            "/v1/base/open-competition-v2-beta2/structured-artifact-profile",
            json=payload,
        )

    def validate_open_competition_v2(self, request: dict):
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST", "/v1/base/open-competition-v2-beta2/validate", json=payload
        )

    def prepare_open_competition_v2_creation(self, request: dict):
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST",
            "/v1/base/open-competition-v2-beta2/creation-preparation",
            json=payload,
        )

    def prepare_open_competition_v2_funding(self, request: dict):
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST",
            "/v1/base/open-competition-v2-beta2/funding-preparation",
            json=payload,
        )

    def list_open_competition_v2_inventory(
        self, network: str = "base-mainnet", state: str | None = None
    ):
        return self._request(
            "GET",
            "/v1/base/open-competition-v2-beta2/inventory",
            params={"network": network, "state": state},
        )

    def list_open_competition_v2_events(
        self, network: str = "base-mainnet", bounty_id: str | None = None
    ):
        return self._request(
            "GET",
            "/v1/base/open-competition-v2-beta2/events",
            params={"network": network, "bounty_id": bounty_id},
        )

    def create_open_competition_v2_proof_quote(self, request: dict):
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST",
            "/v1/base/open-competition-v2-beta2/proof-quotes",
            json=payload,
        )

    def prepare_open_competition_v2_proof(self, request: dict):
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST",
            "/v1/base/open-competition-v2-beta2/proof-preparation",
            json=payload,
        )

    def prepare_open_competition_v2_action(self, request: dict):
        payload = dict(request)
        payload.setdefault("network", "base-mainnet")
        return self._request(
            "POST",
            "/v1/base/open-competition-v2-beta2/action-preparation",
            json=payload,
        )

    def get_open_competition_v2_proof_job(self, job_id: str):
        return self._request(
            "GET", f"/v1/base/open-competition-v2-beta2/proof-jobs/{job_id}"
        )

    def pay_open_competition_v2_proof_job(
        self, job_id: str, payment_signature: str | None = None
    ):
        response = self._http(
            "POST",
            f"/v1/base/open-competition-v2-beta2/proof-jobs/{job_id}/payment",
            headers=(
                {"PAYMENT-SIGNATURE": payment_signature}
                if payment_signature
                else None
            ),
        )
        if response.status_code not in (200, 202, 402, 404, 409, 422, 503):
            response.raise_for_status()
        return {
            "status": response.status_code,
            "payment_required": response.headers.get("PAYMENT-REQUIRED"),
            "payment_response": response.headers.get("PAYMENT-RESPONSE"),
            "body": _x402_response_body(response),
        }

    def authorize_open_competition_v2_proof_relay(
        self,
        job_id: str,
        authorization_deadline: int,
        solver_signature: str | None = None,
    ):
        """Prepare the exact relay signature, then persist the signed authorization."""
        return self._request(
            "POST",
            f"/v1/base/open-competition-v2-beta2/proof-jobs/{job_id}/relay-authorization",
            json={
                "authorization_deadline": authorization_deadline,
                "solver_signature": solver_signature,
            },
        )

    def _standing_meta_v4_action(
        self,
        path: str,
        arguments: dict,
        network: str,
    ):
        return self._request(
            "POST",
            f"/v1/base/standing-meta-v4/{path}",
            json={"network": network, "arguments": arguments},
        )

    def prepare_standing_meta_v4_claim(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "claim-preparation", arguments, network
        )

    def prepare_anonymous_stake_registration(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "stake-registration-preparation", arguments, network
        )

    def set_anonymous_stake_availability(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "stake-availability-preparation", arguments, network
        )

    def list_verification_assignments(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "verification-assignments", arguments, network
        )

    def submit_primary_verdict(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "primary-verdict-preparation", arguments, network
        )

    def waive_verification_appeal(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "appeal-waiver-preparation", arguments, network
        )

    def open_verification_appeal(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "appeal-opening-preparation", arguments, network
        )

    def submit_appeal_vote(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "appeal-vote-preparation", arguments, network
        )

    def finalize_verification_case(
        self, arguments: dict, network: str = "base-mainnet"
    ):
        return self._standing_meta_v4_action(
            "finalization-preparation", arguments, network
        )

    def get_risk_events(
        self,
        action: str | None = None,
        surface: str | None = None,
        bounty_id: str | None = None,
        agent_id: str | None = None,
        limit: int | None = None,
    ):
        return self._request(
            "GET",
            "/v1/risk/events",
            params={
                "action": action,
                "surface": surface,
                "bounty_id": bounty_id,
                "agent_id": agent_id,
                "limit": limit,
            },
        )

    def list_risk_reviews(self):
        return self._request("GET", "/v1/risk/reviews")

    def approve_risk_bounty(
        self,
        risk_event_id: str,
        title: str,
        template_slug: str,
        amount_minor: int,
        currency: str,
        funding_mode: str,
        privacy: str,
        operator_id: str,
        note: str,
    ):
        return self._request(
            "POST",
            "/v1/risk/bounty-approvals",
            json={
                "risk_event_id": risk_event_id,
                "title": title,
                "template_slug": template_slug,
                "amount_minor": amount_minor,
                "currency": currency,
                "funding_mode": funding_mode,
                "privacy": privacy,
                "operator_id": operator_id,
                "note": note,
            },
        )

    def approve_risk_payout(self, risk_event_id: str, operator_id: str, note: str):
        return self._request(
            "POST",
            "/v1/risk/payout-approvals",
            json={
                "risk_event_id": risk_event_id,
                "operator_id": operator_id,
                "note": note,
            },
        )

    def reject_risk_event(self, risk_event_id: str, operator_id: str, note: str):
        return self._request(
            "POST",
            f"/v1/risk/events/{risk_event_id}/reject",
            json={
                "risk_event_id": risk_event_id,
                "operator_id": operator_id,
                "note": note,
            },
        )

    def register_agent(self, handle: str, payout_wallet: str | None = None):
        return self._request(
            "POST",
            "/v1/agents",
            json={"handle": handle, "payout_wallet": payout_wallet},
        )

    def register_capability(
        self,
        agent_id: str,
        capability_class: str,
        template_slugs: list[str],
        min_price_minor: int,
        max_price_minor: int,
        currency: str = "usdc",
        latency_seconds: int = 600,
        supported_verifiers: list[str] | None = None,
    ):
        return self._request(
            "POST",
            "/v1/capabilities",
            json={
                "agent_id": agent_id,
                "class": capability_class,
                "template_slugs": template_slugs,
                "min_price_minor": min_price_minor,
                "max_price_minor": max_price_minor,
                "currency": currency,
                "latency_seconds": latency_seconds,
                "supported_verifiers": supported_verifiers or ["Manual"],
            },
        )

    def create_help_request(
        self,
        requester_agent_id: str,
        goal: str,
        context: str,
        budget_minor: int,
        currency: str = "usdc",
        privacy: str = "Public",
    ):
        return self._request(
            "POST",
            "/v1/help-requests",
            json={
                "requester_agent_id": requester_agent_id,
                "goal": goal,
                "context": context,
                "budget_minor": budget_minor,
                "currency": currency,
                "privacy": privacy,
                "required_confidence": None,
            },
        )

    def request_quotes(self, help_request_id: str):
        return self._request("POST", f"/v1/help-requests/{help_request_id}/quotes", json={})

    def fund_quote_as_bounty(
        self,
        quote_id: str,
        title: str | None = None,
        funding_mode: str | None = None,
    ):
        return self._request(
            "POST",
            f"/v1/quotes/{quote_id}/fund-bounty",
            json={
                "quote_id": quote_id,
                "title": title,
                "funding_mode": funding_mode,
            },
        )

    def post_bounty(
        self,
        title: str,
        template_slug: str,
        amount_minor: int,
        currency: str,
        funding_mode: str,
        privacy: str,
    ):
        return self._request(
            "POST",
            "/v1/bounties",
            json={
                "title": title,
                "template_slug": template_slug,
                "amount_minor": amount_minor,
                "currency": currency,
                "funding_mode": funding_mode,
                "privacy": privacy,
            },
        )

    def open_pooled_bounty(
        self,
        title: str,
        template_slug: str,
        target_amount_minor: int,
        currency: str,
        funding_mode: str,
        privacy: str,
        funding_targets: list[dict] | None = None,
    ):
        return self._request(
            "POST",
            "/v1/bounties/pooled",
            json={
                "title": title,
                "template_slug": template_slug,
                "target_amount_minor": target_amount_minor,
                "currency": currency,
                "funding_mode": funding_mode,
                "privacy": privacy,
                "funding_targets": funding_targets or [],
            },
        )

    def add_funding_contribution(
        self,
        bounty_id: str,
        amount_minor: int,
        currency: str,
        rail: str,
        contributor_agent_id: str | None = None,
        source_organization_id: str | None = None,
        external_reference: str | None = None,
    ):
        return self._request(
            "POST",
            f"/v1/bounties/{bounty_id}/funding-contributions",
            json={
                "bounty_id": bounty_id,
                "contributor_agent_id": contributor_agent_id,
                "source_organization_id": source_organization_id,
                "amount_minor": amount_minor,
                "currency": currency,
                "rail": rail,
                "external_reference": external_reference,
            },
        )

    def create_funding_intent(
        self,
        bounty_id: str,
        amount_minor: int,
        currency: str,
        rail: str,
        contributor_agent_id: str | None = None,
        source_organization_id: str | None = None,
        external_reference: str | None = None,
        stripe_success_url: str | None = None,
        stripe_cancel_url: str | None = None,
        base_escrow_contract: str | None = None,
        base_payer: str | None = None,
        base_token: str | None = None,
        base_network: str | None = None,
    ):
        return self._request(
            "POST",
            f"/v1/bounties/{bounty_id}/funding-intents",
            json={
                "bounty_id": bounty_id,
                "contributor_agent_id": contributor_agent_id,
                "source_organization_id": source_organization_id,
                "amount_minor": amount_minor,
                "currency": currency,
                "rail": rail,
                "external_reference": external_reference,
                "stripe_success_url": stripe_success_url,
                "stripe_cancel_url": stripe_cancel_url,
                "base_escrow_contract": base_escrow_contract,
                "base_payer": base_payer,
                "base_token": base_token,
                "base_network": base_network,
            },
        )

    def list_claimable_bounties(self):
        return self._request("GET", "/v1/bounties/claimable")

    def list_public_bounty_feed(self):
        return self._request("GET", "/v1/bounties/feed")

    def list_public_funding_feed(self):
        return self._request("GET", "/v1/bounties/funding-feed")

    def list_capability_feed(self):
        return self._request("GET", "/v1/capabilities/feed")

    def search_capabilities(
        self,
        capability_class: str | None = None,
        template_slug: str | None = None,
        currency: str | None = None,
        max_price_minor: int | None = None,
    ):
        return self._request(
            "POST",
            "/v1/capabilities/search",
            json={
                "class": capability_class,
                "template_slug": template_slug,
                "currency": currency,
                "max_price_minor": max_price_minor,
            },
        )

    def claim_bounty(self, bounty_id: str, solver_agent_id: str):
        return self._request(
            "POST",
            f"/v1/bounties/{bounty_id}/claim",
            json={"bounty_id": bounty_id, "solver_agent_id": solver_agent_id},
        )

    def submit_result(
        self,
        bounty_id: str,
        solver_agent_id: str,
        artifact_uri: str,
        artifact_body: str,
    ):
        return self._request(
            "POST",
            f"/v1/bounties/{bounty_id}/submit",
            json={
                "bounty_id": bounty_id,
                "solver_agent_id": solver_agent_id,
                "artifact_uri": artifact_uri,
                "artifact_body": artifact_body,
            },
        )

    def request_verification(
        self,
        bounty_id: str,
        submission_id: str,
        expected_artifact_digest: str,
        verifier_kind: str | None = None,
        rubric: str | None = None,
        evidence: dict | None = None,
        approved_risk_event_id: str | None = None,
    ):
        return self._request(
            "POST",
            f"/v1/bounties/{bounty_id}/verify",
            json={
                "bounty_id": bounty_id,
                "submission_id": submission_id,
                "expected_artifact_digest": expected_artifact_digest,
                "verifier_kind": verifier_kind,
                "rubric": rubric,
                "evidence": evidence,
                "approved_risk_event_id": approved_risk_event_id,
            },
        )

    def get_bounty_status(self, bounty_id: str):
        return self._request("GET", f"/v1/bounties/{bounty_id}")

    def get_paid_status(self, bounty_id: str):
        status = self.get_bounty_status(bounty_id)
        return {"bounty_id": bounty_id, "settlements": status.get("settlements", [])}

    def get_agent_paid_status(self, agent_id: str):
        return self._request("GET", f"/v1/agents/{agent_id}/paid-status")

    def publish_autonomous_bounty_terms(self, creator_wallet: str, document: dict):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/terms",
            json={"creator_wallet": creator_wallet, "document": document},
        )

    def get_autonomous_bounty_terms(self, terms_hash: str):
        return self._request(
            "GET", f"/v1/base/autonomous-bounties/terms/{terms_hash}"
        )

    def publish_autonomous_submission_evidence(
        self,
        bounty_contract: str,
        bounty_id: str,
        round: int,
        solver_wallet: str,
        artifact_reference: str,
        evidence: dict,
        network: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/submission-evidence",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "bounty_id": bounty_id,
                "round": round,
                "solver_wallet": solver_wallet,
                "artifact_reference": artifact_reference,
                "evidence": evidence,
            },
        )

    def get_autonomous_submission_evidence(
        self, bounty_contract: str, round: int, network: str | None = None
    ):
        return self._request(
            "GET",
            f"/v1/base/autonomous-bounties/submission-evidence/{bounty_contract}/{round}",
            params={"network": network},
        )

    def list_autonomous_bounties(
        self, network: str | None = None, claimable_only: bool | None = None
    ):
        return self._request(
            "GET",
            "/v1/base/autonomous-bounties/feed",
            params={"network": network, "claimable_only": claimable_only},
        )

    def get_solver_leaderboard(
        self, network: str | None = None, at: str | None = None
    ):
        return self._request(
            "GET",
            "/v1/base/autonomous-bounties/leaderboard",
            params={"network": network, "at": at},
        )

    def list_opportunities(
        self,
        network: str | None = None,
        view: str | None = None,
        source_type: str | None = None,
        work_state: str | None = None,
        payment_state: str | None = None,
        limit: int | None = None,
    ):
        """Read the combined projection without replacing source-of-truth feeds."""
        return self._request(
            "GET",
            "/v1/opportunities",
            params={
                "network": network,
                "view": view,
                "source_type": source_type,
                "work_state": work_state,
                "payment_state": payment_state,
                "limit": limit,
            },
        )

    def analyze_bounty_fit(
        self, bounty_contract: str, network: str | None = None
    ):
        """Return advisory analysis cached by immutable terms hash."""
        return self._request(
            "GET",
            f"/v1/base/autonomous-bounties/{bounty_contract}/analysis",
            params={"network": network},
        )

    def create_discovery_subscription(
        self, endpoint_url: str, filters: dict | None = None
    ):
        """Create a filtered signed webhook; credentials are returned once."""
        return self._request(
            "POST",
            "/v1/discovery/subscriptions",
            json={"endpoint_url": endpoint_url, "filters": filters or {}},
        )

    def get_discovery_subscription(
        self, subscription_id: str, management_token: str
    ):
        return self._request(
            "GET",
            f"/v1/discovery/subscriptions/{subscription_id}",
            headers={"authorization": f"Bearer {management_token}"},
        )

    def delete_discovery_subscription(
        self, subscription_id: str, management_token: str
    ):
        return self._request(
            "DELETE",
            f"/v1/discovery/subscriptions/{subscription_id}",
            headers={"authorization": f"Bearer {management_token}"},
        )

    def get_opportunity_conversion_funnel(self, window_hours: int | None = None):
        """Return observable conversions without inferring agent independence."""
        return self._request(
            "GET",
            "/v1/opportunities/conversion-funnel",
            params={"window_hours": window_hours},
        )

    def get_site_analytics(self, window_hours: int | None = None):
        """Return privacy-minimized browser, channel, and site-action aggregates."""
        return self._request(
            "GET",
            "/v1/analytics/site",
            params={"window_hours": window_hours},
        )

    def list_autonomous_verification_jobs(
        self, network: str | None = None, verifier: str | None = None
    ):
        return self._request(
            "GET",
            "/v1/base/autonomous-bounties/verification-jobs",
            params={"network": network, "verifier": verifier},
        )

    def list_autonomous_bounty_events(
        self, network: str | None = None, bounty_id: str | None = None
    ):
        return self._request(
            "GET",
            "/v1/base/autonomous-bounties/events",
            params={"network": network, "bounty_id": bounty_id},
        )

    def decode_autonomous_bounty_events(self, logs: list[dict]):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/decode-events",
            json={"logs": logs},
        )

    def plan_autonomous_bounty_creation(
        self, create: dict, network: str | None = None
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/creation-plan",
            json={"network": network, "create": create},
        )

    def plan_autonomous_canonical_child_terms(
        self,
        parent_bounty_id: str,
        parent_round: int,
        parent_solver: str,
        parent_solver_reward: dict,
        child_acceptance_criteria: list[str],
        verifier_module: str,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/canonical-child-terms-plan",
            json={
                "parent_bounty_id": parent_bounty_id,
                "parent_round": parent_round,
                "parent_solver": parent_solver,
                "parent_solver_reward": parent_solver_reward,
                "child_acceptance_criteria": child_acceptance_criteria,
                "verifier_module": verifier_module,
            },
        )

    def plan_autonomous_bounty_authorized_creation(
        self,
        create: dict,
        signature: dict,
        network: str | None = None,
        relayer: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/authorized-creation-plan",
            json={
                "network": network,
                "create": create,
                "signature": signature,
                "relayer": relayer,
            },
        )

    def plan_autonomous_bounty_contribution(
        self, contribution: dict, network: str | None = None
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/contribution-plan",
            json={"network": network, "contribution": contribution},
        )

    def plan_autonomous_bounty_authorized_contribution(
        self,
        contribution: dict,
        signature: dict,
        network: str | None = None,
        relayer: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/authorized-contribution-plan",
            json={
                "network": network,
                "contribution": contribution,
                "signature": signature,
                "relayer": relayer,
            },
        )

    def plan_autonomous_bounty_claim(
        self,
        bounty_contract: str,
        solver: str,
        network: str | None = None,
        authorization_nonce: str | None = None,
        authorization_valid_before: int | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/claim-plan",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "solver": solver,
                "authorization_nonce": authorization_nonce,
                "authorization_valid_before": authorization_valid_before,
            },
        )

    def agent_native_claim(
        self,
        bounty_contract: str,
        solver_wallet: str,
        signer: Callable[[dict], str | dict] | None = None,
        *,
        idempotency_key: str | None = None,
        network: str = "base-mainnet",
        agent_id: str | None = None,
        request_bond_sponsorship: bool = False,
        source: str = "sdk-python",
        poll_interval_seconds: float = 1.0,
        timeout_seconds: float = 60.0,
    ):
        """Reserve a claim, optionally sign once, and poll for canonical ownership.

        The signer receives only the server-derived EIP-712 signing_payload and
        should return the wallet's unchanged 65-byte ``0x...`` result. Legacy
        ``{"v": int, "r": "0x...", "s": "0x..."}`` results remain accepted.
        Keys never leave the caller's wallet implementation. When bond
        sponsorship is requested and available, that one signature authorizes
        an atomic bond-plus-claim transaction: either both transitions succeed
        or neither moves value. Only the returned canonical event proves claim
        ownership.
        """
        request = {
            "idempotency_key": idempotency_key or f"sdk-python-{uuid.uuid4()}",
            "network": network,
            "bounty_contract": bounty_contract,
            "solver_wallet": solver_wallet,
            "agent_id": agent_id,
            "request_bond_sponsorship": request_bond_sponsorship,
            "source": source,
        }
        response = self._request(
            "POST", "/v1/base/autonomous-bounties/claims", json=request
        )
        signing_payload = response.get("signing_payload")
        if signer is None or not isinstance(signing_payload, dict):
            return response

        signature = signer(signing_payload)
        if isinstance(signature, str):
            if (
                len(signature) != 132
                or not signature.startswith("0x")
                or not all(character in "0123456789abcdefABCDEF" for character in signature[2:])
            ):
                raise ValueError(
                    "agent claim signer must return one 65-byte 0x-prefixed signature"
                )
            request["wallet_signature"] = signature
        elif isinstance(signature, dict) and {"v", "r", "s"} <= signature.keys():
            request["signature"] = signature
        else:
            raise ValueError(
                "agent claim signer must return a wallet signature or legacy v, r, and s"
            )
        deadline = time.monotonic() + timeout_seconds
        while True:
            response = self._request(
                "POST", "/v1/base/autonomous-bounties/claims", json=request
            )
            candidate = response.get("candidate")
            status = candidate.get("status") if isinstance(candidate, dict) else None
            if status == "claimed":
                if not response.get("canonical_event_id"):
                    raise RuntimeError("claimed response is missing canonical_event_id")
                return response
            if status in {"failed", "superseded", "withdrawn"}:
                raise RuntimeError(f"agent claim ended in terminal state {status}")
            if status == "waitlisted":
                return response
            if time.monotonic() >= deadline:
                raise TimeoutError(
                    "agent claim timed out; replay the same idempotency key and signature"
                )
            time.sleep(poll_interval_seconds)

    def plan_autonomous_bounty_authorized_claim(
        self,
        bounty_contract: str,
        solver: str,
        authorization_nonce: str,
        authorization_valid_before: int,
        signature: dict,
        network: str | None = None,
        relayer: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/authorized-claim-plan",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "solver": solver,
                "authorization_nonce": authorization_nonce,
                "authorization_valid_before": authorization_valid_before,
                "signature": signature,
                "relayer": relayer,
            },
        )

    def plan_autonomous_bounty_submission(
        self,
        bounty_contract: str,
        solver: str,
        submission_hash: str,
        evidence_hash: str,
        network: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/submission-plan",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "solver": solver,
                "submission_hash": submission_hash,
                "evidence_hash": evidence_hash,
            },
        )

    def prepare_autonomous_bounty_submission(
        self,
        bounty_contract: str,
        solver_wallet: str,
        artifact_reference: str,
        evidence: dict,
        network: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/submission-preparation",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "solver_wallet": solver_wallet,
                "artifact_reference": artifact_reference,
                "evidence": evidence,
            },
        )

    def plan_autonomous_bounty_submission_authorization(
        self, submission: dict, network: str | None = None
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/submission-authorization-plan",
            json={"network": network, "submission": submission},
        )

    def plan_autonomous_verification_attestation(
        self, attestation: dict, network: str | None = None
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/verification-attestation-plan",
            json={"network": network, "attestation": attestation},
        )

    def plan_autonomous_module_settlement(
        self,
        bounty_contract: str,
        proof: str,
        network: str | None = None,
        caller: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/module-settlement-plan",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "caller": caller,
                "proof": proof,
            },
        )

    def plan_autonomous_attestation_settlement(
        self,
        bounty_contract: str,
        attestations: list[dict],
        network: str | None = None,
        caller: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/autonomous-bounties/attestation-settlement-plan",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "caller": caller,
                "attestations": attestations,
            },
        )

    def _plan_autonomous_lifecycle(
        self,
        action: str,
        bounty_contract: str,
        network: str | None = None,
        caller: str | None = None,
    ):
        return self._request(
            "POST",
            f"/v1/base/autonomous-bounties/{action}-plan",
            json={
                "network": network,
                "bounty_contract": bounty_contract,
                "caller": caller,
            },
        )

    def plan_autonomous_expire_claim(self, bounty_contract: str, **kwargs):
        return self._plan_autonomous_lifecycle("expire-claim", bounty_contract, **kwargs)

    def plan_autonomous_expire_submission(self, bounty_contract: str, **kwargs):
        return self._plan_autonomous_lifecycle(
            "expire-submission", bounty_contract, **kwargs
        )

    def plan_autonomous_cancel(self, bounty_contract: str, **kwargs):
        return self._plan_autonomous_lifecycle("cancel", bounty_contract, **kwargs)

    def delete_unclaimed_bounty(
        self,
        bounty_contract: str,
        creator_wallet: str,
        network: str | None = None,
    ):
        return self._plan_autonomous_lifecycle(
            "cancel",
            bounty_contract,
            network=network,
            caller=creator_wallet,
        )

    def plan_autonomous_refund_withdrawal(self, bounty_contract: str, **kwargs):
        return self._plan_autonomous_lifecycle(
            "refund-withdrawal", bounty_contract, **kwargs
        )

    def broadcast_base_signed_transaction(
        self,
        signed_transaction: str,
        request_id: int | None = None,
        network: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/broadcast-signed-transaction",
            json={
                "signed_transaction": signed_transaction,
                "request_id": request_id,
                "network": network,
            },
        )

    def get_base_transaction_receipt(
        self,
        tx_hash: str,
        request_id: int | None = None,
        network: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/base/transaction-receipt",
            json={
                "tx_hash": tx_hash,
                "request_id": request_id,
                "network": network,
            },
        )

    def plan_stripe_checkout_top_up(
        self,
        organization_id: str,
        amount_minor: int,
        currency: str = "usd",
        success_url: str | None = None,
        cancel_url: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/stripe/checkout-top-ups",
            json={
                "organization_id": organization_id,
                "amount_minor": amount_minor,
                "currency": currency,
                "success_url": success_url,
                "cancel_url": cancel_url,
            },
        )

    def plan_stripe_connect_account(self, agent_id: str):
        return self._request(
            "POST",
            "/v1/stripe/connect-accounts",
            json={"agent_id": agent_id},
        )

    def plan_stripe_connect_transfer(
        self,
        payout_intent_id: str,
        connected_account_id: str,
    ):
        return self._request(
            "POST",
            "/v1/stripe/connect-transfers",
            json={
                "payout_intent_id": payout_intent_id,
                "connected_account_id": connected_account_id,
            },
        )

    def execute_stripe_checkout_top_up(
        self,
        organization_id: str,
        amount_minor: int,
        currency: str = "usd",
        success_url: str | None = None,
        cancel_url: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/stripe/live/checkout-top-ups",
            json={
                "organization_id": organization_id,
                "amount_minor": amount_minor,
                "currency": currency,
                "success_url": success_url,
                "cancel_url": cancel_url,
            },
        )

    def execute_stripe_funding_intent_checkout(self, funding_intent_id: str):
        return self._request(
            "POST",
            f"/v1/stripe/live/funding-intents/{funding_intent_id}/checkout-session",
        )

    def execute_stripe_connect_account(self, agent_id: str):
        return self._request(
            "POST",
            "/v1/stripe/live/connect-accounts",
            json={"agent_id": agent_id},
        )

    def execute_stripe_connect_transfer(
        self,
        payout_intent_id: str,
        connected_account_id: str,
    ):
        return self._request(
            "POST",
            "/v1/stripe/live/connect-transfers",
            json={
                "payout_intent_id": payout_intent_id,
                "connected_account_id": connected_account_id,
            },
        )

    def plan_github_issue_bounty(
        self,
        repository: str,
        issue_url: str,
        title: str,
        body: str,
    ):
        return self._request(
            "POST",
            "/v1/github/issue-bounty-plan",
            json={
                "repository": repository,
                "issue_url": issue_url,
                "title": title,
                "body": body,
            },
        )

    def plan_github_funding_comment(
        self,
        repository: str,
        issue_url: str,
        title: str,
        body: str,
        comment_body: str,
        contributor_login: str | None = None,
        comment_id: str | None = None,
        existing_idempotency_keys: list[str] | None = None,
    ):
        return self._request(
            "POST",
            "/v1/github/funding-comment-plan",
            json={
                "repository": repository,
                "issue_url": issue_url,
                "title": title,
                "body": body,
                "comment_body": comment_body,
                "contributor_login": contributor_login,
                "comment_id": comment_id,
                "existing_idempotency_keys": existing_idempotency_keys or [],
            },
        )

    def plan_github_claim_comment(
        self,
        repository: str,
        issue_url: str,
        title: str,
        body: str,
        comment_body: str,
        contributor_login: str | None = None,
        comment_id: str | None = None,
        claim_age_minutes: int | None = None,
        progress_signal_count: int = 0,
        active_claim_login: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/github/claim-comment-plan",
            json={
                "repository": repository,
                "issue_url": issue_url,
                "title": title,
                "body": body,
                "comment_body": comment_body,
                "contributor_login": contributor_login,
                "comment_id": comment_id,
                "claim_age_minutes": claim_age_minutes,
                "progress_signal_count": progress_signal_count,
                "active_claim_login": active_claim_login,
            },
        )

    def plan_github_proof_comment(
        self,
        bounty_id: str,
        proof_url: str,
        verifier_summary: str,
        settlement_url: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/github/proof-comment-plan",
            json={
                "bounty_id": bounty_id,
                "proof_url": proof_url,
                "verifier_summary": verifier_summary,
                "settlement_url": settlement_url,
            },
        )

    def plan_github_proof_comment_from_proof(
        self,
        proof_id: str,
        settlement_url: str | None = None,
    ):
        return self._request(
            "POST",
            "/v1/github/proof-comment-plan-from-proof",
            json={
                "proof_id": proof_id,
                "settlement_url": settlement_url,
            },
        )

    def reconcile_stripe_connect_snapshot(self, snapshot: dict):
        return self._request("POST", "/v1/stripe/connect-snapshots", json=snapshot)

    def reconcile_stripe_checkout_webhook(
        self,
        event: dict,
        stripe_signature: str | None = None,
    ):
        return self._stripe_event(
            "/v1/stripe/checkout-webhooks", event, stripe_signature
        )

    def reconcile_stripe_transfer_event(
        self,
        event: dict,
        stripe_signature: str | None = None,
    ):
        return self._stripe_event("/v1/stripe/transfer-events", event, stripe_signature)

    def run_bountybench(self):
        return self._request("GET", "/v1/evals/bountybench")

    def run_abusebench(self):
        return self._request("GET", "/v1/evals/abusebench")

    def run_judgebench(self):
        return self._request("GET", "/v1/evals/judgebench")

    def run_eval_loops(self):
        return self._request("GET", "/v1/evals/loops")

    def get_eval_runs(self):
        return self._request("GET", "/v1/evals/runs")
