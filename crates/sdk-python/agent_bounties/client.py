import hashlib
import os

import httpx


def hash_artifact(body: str) -> str:
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


class AgentBountiesClient:
    def __init__(
        self,
        base_url: str = "http://127.0.0.1:8080",
        operator_api_token: str | None = None,
    ):
        self.base_url = base_url.rstrip("/")
        self.operator_api_token = operator_api_token or os.getenv("OPERATOR_API_TOKEN")

    def _headers(self) -> dict[str, str] | None:
        if self.operator_api_token:
            return {"x-operator-token": self.operator_api_token}
        return None

    def _request(
        self,
        method: str,
        path: str,
        json: dict | None = None,
        params: dict | None = None,
    ):
        query = (
            {key: value for key, value in params.items() if value is not None}
            if params
            else None
        )
        response = httpx.request(
            method,
            f"{self.base_url}{path}",
            json=json,
            params=query,
            headers=self._headers(),
            timeout=30,
        )
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

    def get_risk_policy(self):
        return self._request("GET", "/v1/risk/policy")

    def get_live_money_readiness(self, network: str | None = None):
        return self._request(
            "GET",
            "/v1/readiness/live-money",
            params={"network": network},
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
        headers = self._headers() or {}
        if stripe_signature:
            headers["stripe-signature"] = stripe_signature
        response = httpx.request(
            "POST",
            f"{self.base_url}/v1/stripe/checkout-webhooks",
            json=event,
            headers=headers or None,
            timeout=30,
        )
        response.raise_for_status()
        return response.json()

    def reconcile_stripe_transfer_event(
        self,
        event: dict,
        stripe_signature: str | None = None,
    ):
        headers = self._headers() or {}
        if stripe_signature:
            headers["stripe-signature"] = stripe_signature
        response = httpx.request(
            "POST",
            f"{self.base_url}/v1/stripe/transfer-events",
            json=event,
            headers=headers or None,
            timeout=30,
        )
        response.raise_for_status()
        return response.json()

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
