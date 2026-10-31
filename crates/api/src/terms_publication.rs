//! One authenticated boundary for direct and child terms publication.
use crate::{
    agent_action_error, distribution, secure_operations, security_auth, AgentActionApiError,
    SharedState,
};
use axum::http::{HeaderMap, StatusCode};
use db::terms_publication::{
    OwnedTermsPublication, TermsAttribution, TermsPublicationError, TermsPublicationIdentity,
    TermsPublicationOperation,
};
use domain::AutonomousBountyTermsRecord;
use serde_json::{json, Value};
use service_runtime::auth;

pub struct PublicationContext {
    principal: String,
    network: String,
    creator: String,
    operation: TermsPublicationOperation,
    key_hash: String,
    payload_hash: String,
    attribution: Option<TermsAttribution>,
}

fn problem(status: StatusCode, code: &str, message: &str, retryable: bool) -> AgentActionApiError {
    agent_action_error(status, code, message, retryable,
        "Use a website account with this verified creator wallet or a wallet session. Keep the same Idempotency-Key and exact content when retrying; no wallet payment is requested by publication.")
}

pub fn auth_problem(status: StatusCode) -> AgentActionApiError {
    match status {
        StatusCode::UNAUTHORIZED => problem(status,"publication_authentication_required","Sign in before publishing terms.",false),
        StatusCode::FORBIDDEN => problem(status,"publication_wallet_authority_required","Verify control of the document's creator wallet on its network. An operator credential or connected wallet string is not wallet authority.",false),
        StatusCode::BAD_REQUEST => problem(status,"invalid_publication_request","Use one valid credential and Idempotency-Key; the request creator and document network/creator must agree.",false),
        _ => problem(StatusCode::SERVICE_UNAVAILABLE,"publication_authentication_unavailable","Publication authority could not be verified. Keep the draft and retry later.",true),
    }
}

impl PublicationContext {
    pub async fn authenticate(
        state: &SharedState,
        headers: &HeaderMap,
        operation: TermsPublicationOperation,
        network: &str,
        creator: &str,
        request: Value,
    ) -> Result<Self, AgentActionApiError> {
        service_runtime::terms_publication::validate_headers(headers).map_err(|e| {
            auth_problem(if matches!(e, auth::AuthError::Unauthorized) {
                StatusCode::FORBIDDEN
            } else {
                security_auth::status(e)
            })
        })?;
        let principal = secure_operations::authenticated(state, headers)
            .await
            .map_err(auth_problem)?;
        security_auth::authorize_verified_wallet(state, &principal, creator, network)
            .await
            .map_err(auth_problem)?;
        let key = service_runtime::terms_publication::idempotency_key(headers)
            .map_err(|e| auth_problem(security_auth::status(e)))?;
        let attribution = distribution::authenticated_handoff(state, headers)
            .map_err(auth_problem)?
            .map(|(token_hash, handoff_id)| TermsAttribution {
                token_hash,
                handoff_id,
            });
        let fingerprint = json!({"network":network,"creator":auth::normalize_wallet(creator).map_err(|e|auth_problem(security_auth::status(e)))?,"request":request,
            "attribution":attribution.as_ref().map(|a|json!({"token_hash":a.token_hash,"handoff_id":a.handoff_id}))});
        let digest = chain_base::keccak256_canonical_json(&fingerprint)
            .map_err(|_| auth_problem(StatusCode::BAD_REQUEST))?;
        Ok(Self {
            network: network.into(),
            creator: auth::normalize_wallet(creator)
                .map_err(|e| auth_problem(security_auth::status(e)))?,
            principal: principal
                .key()
                .ok_or_else(|| auth_problem(StatusCode::UNAUTHORIZED))?,
            operation,
            key_hash: auth::hash_secret(key),
            payload_hash: digest.trim_start_matches("0x").into(),
            attribution,
        })
    }
    pub fn bind_parent(&mut self, context: Value) -> Result<(), AgentActionApiError> {
        self.payload_hash = chain_base::keccak256_canonical_json(
            &json!({"request_hash":self.payload_hash,"parent":context}),
        )
        .map_err(|_| auth_problem(StatusCode::BAD_REQUEST))?
        .trim_start_matches("0x")
        .into();
        Ok(())
    }
    fn identity(&self) -> TermsPublicationIdentity<'_> {
        TermsPublicationIdentity {
            principal: &self.principal,
            operation: self.operation,
            key_hash: &self.key_hash,
            payload_hash: &self.payload_hash,
        }
    }
    pub async fn replay(
        &self,
        state: &SharedState,
    ) -> Result<Option<AutonomousBountyTermsRecord>, AgentActionApiError> {
        let store = state
            .store
            .as_ref()
            .ok_or_else(|| auth_problem(StatusCode::SERVICE_UNAVAILABLE))?;
        store
            .lookup_owned_terms_publication(self.identity(), self.attribution.as_ref())
            .await
            .map_err(store_problem)
    }
    pub async fn reject_existing(
        &self,
        state: &SharedState,
        document: &domain::AutonomousBountyTermsDocument,
    ) -> Result<(), AgentActionApiError> {
        let store = state
            .store
            .as_ref()
            .ok_or_else(|| auth_problem(StatusCode::SERVICE_UNAVAILABLE))?;
        // Lookup must use the same document bytes as the original builder, even
        // after its funding cutoff. Keep the request fingerprint unchanged.
        let mut document = document.clone();
        chain_base::normalize_autonomous_terms_claim_metadata(&mut document)
            .map_err(|_| auth_problem(StatusCode::BAD_REQUEST))?;
        let value =
            serde_json::to_value(&document).map_err(|_| auth_problem(StatusCode::BAD_REQUEST))?;
        let terms_hash = chain_base::keccak256_canonical_json(&value)
            .map_err(|_| auth_problem(StatusCode::BAD_REQUEST))?;
        if let Some(existing) = store
            .get_autonomous_bounty_terms(&terms_hash)
            .await
            .map_err(|_| auth_problem(StatusCode::SERVICE_UNAVAILABLE))?
        {
            if existing.document != document
                || !existing.creator_wallet.eq_ignore_ascii_case(&self.creator)
            {
                return Err(store_problem(TermsPublicationError::Conflict));
            }
            return Err(store_problem(TermsPublicationError::AlreadyExists {
                terms_hash,
            }));
        }
        Ok(())
    }
    pub async fn publish(
        &self,
        state: &SharedState,
        record: &AutonomousBountyTermsRecord,
    ) -> Result<AutonomousBountyTermsRecord, AgentActionApiError> {
        let identity = service_runtime::terms_publication::document_identity(
            &record.document,
            &record.creator_wallet,
        )
        .map_err(|e| auth_problem(security_auth::status(e)))?;
        if identity != (self.network.clone(), self.creator.clone()) {
            return Err(auth_problem(StatusCode::FORBIDDEN));
        }
        let store = state
            .store
            .as_ref()
            .ok_or_else(|| auth_problem(StatusCode::SERVICE_UNAVAILABLE))?;
        let saved = store
            .publish_owned_terms(OwnedTermsPublication {
                identity: self.identity(),
                record,
                daily_limit: secure_operations::limit("AUTH_PUBLICATIONS_PER_DAY", 3),
                attribution: self.attribution.as_ref(),
            })
            .await
            .map_err(store_problem)?;
        if saved.terms_hash != record.terms_hash {
            return Err(store_problem(TermsPublicationError::Conflict));
        }
        Ok(saved)
    }
}

fn store_problem(error: TermsPublicationError) -> AgentActionApiError {
    match error {
        TermsPublicationError::AlreadyExists { terms_hash } => {
            let mut error=problem(StatusCode::CONFLICT,"publication_already_exists","These exact terms already exist. Read the canonical record; a new key cannot replace its original publication receipt.",false);
            error.1.0.next_action_url=Some(format!("/v1/base/autonomous-bounties/terms/{terms_hash}"));
            error
        }
        TermsPublicationError::Conflict => problem(StatusCode::CONFLICT,"publication_idempotency_conflict","This key already identifies different publication content. Preserve the original result.",false),
        TermsPublicationError::AttributionConflict => problem(StatusCode::CONFLICT,"distribution_attribution_binding_failed","The supplied attribution does not match this exact publication. No new publication was committed.",false),
        TermsPublicationError::Auth(db::auth::AuthStoreError::QuotaExceeded) => problem(StatusCode::TOO_MANY_REQUESTS,"publication_limit_reached","The daily publication limit has been reached. Existing exact retries remain available.",false),
        _ => problem(StatusCode::SERVICE_UNAVAILABLE,"terms_store_unavailable","Publication could not be confirmed. Retry the same key and content before any wallet action.",true),
    }
}

#[cfg(test)]
mod tests;
