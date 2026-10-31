//! Fresh, unsigned authority to initially submit one immutable saved envelope.
//! This module has no signer and never returns signed bytes.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedNonceDispatch {
    pub operation_id: Uuid,
    pub retirement_plan_id: Uuid,
    pub retirement_digest: String,
    pub transaction_hash: String,
    pub envelope_sha256: String,
    pub maximum_cost_wei: u128,
    pub provider_hashes: [String; 2],
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

fn sha256_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl RetainedNonceDispatch {
    pub fn validate(&self) -> Result<(), ChainBaseError> {
        require(!self.operation_id.is_nil() && !self.retirement_plan_id.is_nil())?;
        require(normalize_hash(&self.retirement_digest)? == self.retirement_digest)?;
        require(normalize_hash(&self.transaction_hash)? == self.transaction_hash)?;
        require(sha256_valid(&self.envelope_sha256))?;
        require(self.provider_hashes.iter().all(|h| sha256_valid(h)))?;
        require(self.provider_hashes[0] != self.provider_hashes[1])?;
        require(self.maximum_cost_wei > 0 && self.created_at.timestamp() >= 0)?;
        require(
            self.expires_at > self.created_at
                && self.expires_at - self.created_at <= chrono::Duration::minutes(15),
        )
    }

    pub fn digest(&self) -> Result<String, ChainBaseError> {
        self.validate()?;
        let mut bytes = b"agent-bounties/retained-nonce-dispatch-v1\0".to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(|_| denied())?);
        Ok(format!("0x{}", hex::encode(keccak_word(&bytes))))
    }

    /// Structural binding does not grant temporal or dispatch authority.
    pub fn bind(
        &self,
        plan_id: Uuid,
        plan: &NonceRetirementPlan,
        replacement: &PreparedBaseRelayTransaction,
    ) -> Result<(), ChainBaseError> {
        self.validate()?;
        validate_nonce_retirement_envelope(plan, replacement)?;
        require(
            self.operation_id == plan.operation_id
                && self.created_at >= plan.created_at
                && self.retirement_plan_id == plan_id
                && self.retirement_digest == plan.digest()?
                && self.transaction_hash == replacement.transaction.tx_hash
                && self.maximum_cost_wei == plan.maximum_cost_wei
                && self.envelope_sha256
                    == hex::encode(Sha256::digest(parse_alloy_bytes(
                        &replacement.signed_transaction,
                    )?)),
        )
    }

    pub fn propose(
        urls: [&str; 2],
        plan_id: Uuid,
        plan: &NonceRetirementPlan,
        replacement: &PreparedBaseRelayTransaction,
        now: DateTime<Utc>,
    ) -> Result<Self, ChainBaseError> {
        let proposal = Self {
            operation_id: plan.operation_id,
            retirement_plan_id: plan_id,
            retirement_digest: plan.digest()?,
            transaction_hash: replacement.transaction.tx_hash.clone(),
            envelope_sha256: hex::encode(Sha256::digest(parse_alloy_bytes(
                &replacement.signed_transaction,
            )?)),
            maximum_cost_wei: plan.maximum_cost_wei,
            provider_hashes: providers(urls)?,
            created_at: now,
            expires_at: now + chrono::Duration::minutes(15),
        };
        proposal.bind(plan_id, plan, replacement)?;
        Ok(proposal)
    }
}

fn dispatch_deadline_error(_: tokio::time::error::Elapsed) -> ChainBaseError {
    // An incomplete evidence read is unavailable, not proof of a policy failure.
    ChainBaseError::RpcTransport("retained dispatch evidence deadline elapsed".into())
}

#[allow(clippy::too_many_arguments)]
pub async fn check_retained_dispatch(
    urls: [&str; 2],
    proposal: &RetainedNonceDispatch,
    plan_id: Uuid,
    plan: &NonceRetirementPlan,
    original: &PreparedBaseRelayTransaction,
    replacement: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    seconds: u64,
) -> Result<NonceRetirementObservation, ChainBaseError> {
    let t = ReqwestJsonRpcTransport::without_redirects(std::time::Duration::from_secs(3))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(seconds.min(90)),
        check_retained_dispatch_with_transport(
            urls,
            proposal,
            plan_id,
            plan,
            original,
            replacement,
            scope,
            policy,
            Utc::now(),
            &t,
        ),
    )
    .await
    .map_err(dispatch_deadline_error)?
}

#[allow(clippy::too_many_arguments)]
pub async fn check_retained_dispatch_with_transport<T: JsonRpcTransport + ?Sized>(
    urls: [&str; 2],
    proposal: &RetainedNonceDispatch,
    plan_id: Uuid,
    plan: &NonceRetirementPlan,
    original: &PreparedBaseRelayTransaction,
    replacement: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    now: DateTime<Utc>,
    t: &T,
) -> Result<NonceRetirementObservation, ChainBaseError> {
    proposal.bind(plan_id, plan, replacement)?;
    require(proposal.provider_hashes == providers(urls)?)?;
    require(now >= proposal.created_at && now < proposal.expires_at)?;
    let started = std::time::Instant::now();
    let observation = check_evidence_with_transport(
        urls,
        plan,
        original,
        scope,
        policy,
        now,
        proposal.expires_at,
        t,
    )
    .await?;
    // Saved bytes are custody evidence, not evidence of absence from a provider.
    for url in urls {
        for method in ["eth_getTransactionByHash", "eth_getTransactionReceipt"] {
            require(
                read(t, url, method, json!([proposal.transaction_hash]))
                    .await?
                    .is_null(),
            )?;
        }
    }
    require(
        now + chrono::Duration::from_std(started.elapsed()).map_err(|_| denied())?
            < proposal.expires_at,
    )?;
    Ok(observation)
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[tokio::test]
    async fn elapsed_retained_dispatch_check_is_unavailable_not_policy_rejection() {
        // This future cannot perform a read, sign, or send. A real Tokio timer
        // supplies the same elapsed error as the bounded production check.
        let elapsed = tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>())
            .await
            .unwrap_err();
        assert_eq!(
            dispatch_deadline_error(elapsed),
            ChainBaseError::RpcTransport("retained dispatch evidence deadline elapsed".into())
        );
    }

    #[tokio::test]
    async fn retained_dispatch_deadline_preserves_completed_checks() {
        for outcome in 0..3 {
            let expected = || match outcome {
                0 => Ok(7_u64),
                1 => Err(denied()),
                _ => Err(ChainBaseError::RpcRetryAfter {
                    status: 429,
                    retry_after_seconds: 9,
                }),
            };
            let observed =
                tokio::time::timeout(std::time::Duration::ZERO, std::future::ready(expected()))
                    .await
                    .map_err(dispatch_deadline_error)
                    .expect("an immediately completed check must keep its result");
            assert_eq!(observed, expected());
        }
    }
}
