//! Authenticated hosted publication persistence. Wallet proof belongs to the caller boundary.
//! This transaction never changes funded terms or infers ownership of historical records.
use crate::{auth, AutonomousBountyTermsRecord, DbError, DistributionHandoff, PostgresStore};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub enum TermsPublicationOperation {
    Terms,
    RoutedChild,
}
impl TermsPublicationOperation {
    fn name(self) -> &'static str {
        match self {
            Self::Terms => "autonomous_terms_publication",
            Self::RoutedChild => "routed_child_publication",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TermsPublicationIdentity<'a> {
    pub principal: &'a str,
    pub operation: TermsPublicationOperation,
    pub key_hash: &'a str,
    pub payload_hash: &'a str,
}

#[derive(Debug, Clone)]
pub struct TermsAttribution {
    pub token_hash: String,
    pub handoff_id: Uuid,
}

pub struct OwnedTermsPublication<'a> {
    pub identity: TermsPublicationIdentity<'a>,
    pub record: &'a AutonomousBountyTermsRecord,
    pub daily_limit: i64,
    pub attribution: Option<&'a TermsAttribution>,
}

#[derive(Debug, thiserror::Error)]
pub enum TermsPublicationError {
    #[error("publication key or immutable content conflicts")]
    Conflict,
    #[error("publication already exists")]
    AlreadyExists { terms_hash: String },
    #[error("publication attribution conflicts")]
    AttributionConflict,
    #[error(transparent)]
    Auth(#[from] auth::AuthStoreError),
    #[error("publication store unavailable")]
    Store(#[from] sqlx::Error),
    #[error("publication store unavailable")]
    Db(#[from] DbError),
    #[error("publication receipt unavailable")]
    Json(#[from] serde_json::Error),
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn identity_hashes(
    identity: TermsPublicationIdentity<'_>,
    attribution: Option<&TermsAttribution>,
) -> Result<(String, String), TermsPublicationError> {
    let principal = identity.principal;
    if !(principal.starts_with("account:") || principal.starts_with("wallet:"))
        || principal.len() > 200
        || principal.chars().any(char::is_control)
        || !digest(identity.key_hash)
        || !digest(identity.payload_hash)
        || attribution.is_some_and(|binding| !digest(&binding.token_hash))
    {
        return Err(auth::AuthStoreError::InvalidInput.into());
    }
    let key = hex::encode(Sha256::digest(format!(
        "autonomous-publication-key-v1\0{}\0{}",
        identity.operation.name(),
        identity.key_hash
    )));
    // Bind attribution independently of the HTTP caller's fingerprint implementation.
    // Only token hashes enter storage; no acquisition token or session is retained.
    let binding = attribution.map(|a| (&a.token_hash, a.handoff_id));
    let payload = hex::encode(Sha256::digest(serde_json::to_vec(&(
        "autonomous-publication-request-v1",
        identity.operation.name(),
        identity.payload_hash,
        binding,
    ))?));
    Ok((key, payload))
}

fn same_immutable_record(
    first: &AutonomousBountyTermsRecord,
    second: &AutonomousBountyTermsRecord,
) -> Result<bool, serde_json::Error> {
    let mut a = serde_json::to_value(first)?;
    let mut b = serde_json::to_value(second)?;
    a.as_object_mut().unwrap().remove("created_at");
    b.as_object_mut().unwrap().remove("created_at");
    Ok(a == b)
}

fn check_record(record: &AutonomousBountyTermsRecord) -> Result<(), TermsPublicationError> {
    let hashed = |value: &serde_json::Value| {
        chain_base::keccak256_canonical_json(value).map_err(|_| TermsPublicationError::Conflict)
    };
    let document = serde_json::to_value(&record.document)?;
    if record.terms_hash != hashed(&document)?
        || record.policy_hash != hashed(&record.document.verification_policy)?
        || record.acceptance_criteria_hash != hashed(&document["acceptance_criteria"])?
        || record.benchmark_hash != hashed(&record.document.benchmark)?
        || record.evidence_schema_hash != hashed(&record.document.evidence_schema)?
        || record.document.contract_terms["creator_wallet"]
            .as_str()
            .is_none_or(|wallet| !wallet.eq_ignore_ascii_case(&record.creator_wallet))
    {
        return Err(TermsPublicationError::Conflict);
    }
    Ok(())
}

async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    identity: TermsPublicationIdentity<'_>,
    key: &str,
    payload: &str,
) -> Result<Option<AutonomousBountyTermsRecord>, TermsPublicationError> {
    let row = sqlx::query("SELECT payload_hash,result_json FROM security_mutation_results WHERE principal_key=$1 AND operation=$2 AND idempotency_hash=$3")
        .bind(identity.principal).bind(identity.operation.name()).bind(key)
        .fetch_optional(&mut **tx).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.try_get::<String, _>("payload_hash")? != payload {
        return Err(TermsPublicationError::Conflict);
    }
    let original: AutonomousBountyTermsRecord =
        serde_json::from_value(row.try_get("result_json")?)?;
    check_record(&original)?;
    let current = sqlx::query("SELECT * FROM autonomous_bounty_terms WHERE terms_hash=$1")
        .bind(&original.terms_hash)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(TermsPublicationError::Conflict)?;
    let current = crate::autonomous_terms_from_row(current)?;
    if serde_json::to_value(&current)? != serde_json::to_value(&original)? {
        return Err(TermsPublicationError::Conflict);
    }
    Ok(Some(original))
}

impl PostgresStore {
    /// Read only: caller must authenticate first. Does not repeat fresh admission rules.
    pub async fn lookup_owned_terms_publication(
        &self,
        identity: TermsPublicationIdentity<'_>,
        attribution: Option<&TermsAttribution>,
    ) -> Result<Option<AutonomousBountyTermsRecord>, TermsPublicationError> {
        let (key, payload) = identity_hashes(identity, attribution)?;
        let mut tx = self.pool.begin().await?;
        let result = replay(&mut tx, identity, &key, &payload).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn publish_owned_terms(
        &self,
        request: OwnedTermsPublication<'_>,
    ) -> Result<AutonomousBountyTermsRecord, TermsPublicationError> {
        let identity = request.identity;
        let (key, payload) = identity_hashes(identity, request.attribution)?;
        check_record(request.record)?;
        if !(1..=10000).contains(&request.daily_limit) {
            return Err(auth::AuthStoreError::InvalidInput.into());
        }
        let mut tx = self.pool.begin().await?;
        // Shared order: existing publication-principal lock, exact hash lock, quota lock.
        // The principal lock matches create_owned_trial_bounty; the quota helper also
        // serializes its shared action bucket across every service instance.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("publication:{}", identity.principal))
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,67280253))")
            .bind(&request.record.terms_hash)
            .execute(&mut *tx)
            .await?;
        if let Some(original) = replay(&mut tx, identity, &key, &payload).await? {
            if !same_immutable_record(&original, request.record)? {
                return Err(TermsPublicationError::Conflict);
            }
            tx.commit().await?;
            return Ok(original);
        }
        let existing = sqlx::query("SELECT * FROM autonomous_bounty_terms WHERE terms_hash=$1")
            .bind(&request.record.terms_hash)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(existing) = existing {
            let original = crate::autonomous_terms_from_row(existing)?;
            if !same_immutable_record(&original, request.record)? {
                return Err(TermsPublicationError::Conflict);
            }
            return Err(TermsPublicationError::AlreadyExists {
                terms_hash: original.terms_hash,
            });
        }
        auth::consume_security_quota_in_transaction(
            &mut tx,
            identity.principal,
            "publication",
            &key,
            &payload,
            request.daily_limit,
            86400,
        )
        .await?;
        let record = request.record;
        let inserted = sqlx::query("INSERT INTO autonomous_bounty_terms(terms_hash,policy_hash,acceptance_criteria_hash,benchmark_hash,evidence_schema_hash,creator_wallet,document,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(terms_hash) DO NOTHING RETURNING *")
            .bind(&record.terms_hash).bind(&record.policy_hash).bind(&record.acceptance_criteria_hash)
            .bind(&record.benchmark_hash).bind(&record.evidence_schema_hash).bind(&record.creator_wallet)
            .bind(serde_json::to_value(&record.document)?).bind(record.created_at)
            .fetch_optional(&mut *tx).await?;
        // A trusted internal writer may not use the public hash lock. Never overwrite
        // its record or retain quota if it won the unique-key race.
        let saved = crate::autonomous_terms_from_row(inserted.ok_or_else(|| {
            TermsPublicationError::AlreadyExists {
                terms_hash: record.terms_hash.clone(),
            }
        })?)?;
        auth::claim_resource_ownership_in_transaction(
            &mut tx,
            "autonomous_terms",
            &saved.terms_hash,
            identity.principal,
        )
        .await?;
        if let Some(attribution) = request.attribution {
            bind_distribution_handoff_terms_in_transaction(
                &mut tx,
                &attribution.token_hash,
                attribution.handoff_id,
                &saved.terms_hash,
                &saved.creator_wallet,
                Utc::now(),
            )
            .await
            .map_err(|error| match error {
                DbError::DistributionAttributionConflict(_) => {
                    TermsPublicationError::AttributionConflict
                }
                other => TermsPublicationError::Db(other),
            })?;
        }
        sqlx::query("INSERT INTO security_mutation_results(principal_key,operation,idempotency_hash,payload_hash,result_json) VALUES($1,$2,$3,$4,$5)")
            .bind(identity.principal).bind(identity.operation.name()).bind(key).bind(payload)
            .bind(serde_json::to_value(&saved)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(saved)
    }
}

/// Existing attribution semantics, composable with the caller's publication transaction.
pub async fn bind_distribution_handoff_terms_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    token_hash: &str,
    handoff_id: Uuid,
    terms_hash: &str,
    creator_wallet: &str,
    bound_at: DateTime<Utc>,
) -> Result<DistributionHandoff, DbError> {
    let terms_hash = terms_hash.trim().to_ascii_lowercase();
    let creator_wallet = creator_wallet.trim().to_ascii_lowercase();
    let row = sqlx::query(
        r#"
        UPDATE distribution_acquisition_handoffs AS handoff
        SET terms_hash = COALESCE(handoff.terms_hash, $3),
            creator_wallet = COALESCE(handoff.creator_wallet, $4),
            terms_bound_at = COALESCE(handoff.terms_bound_at, $5)
        WHERE handoff.id = $1
          AND handoff.acquisition_id = (
            SELECT acquisition.id
            FROM distribution_acquisitions AS acquisition
            WHERE acquisition.token_hash = $2
          )
          AND (handoff.terms_hash IS NULL OR handoff.terms_hash = $3)
          AND (handoff.creator_wallet IS NULL OR handoff.creator_wallet = $4)
        RETURNING id, acquisition_id, request_fingerprint, terms_hash,
                  creator_wallet, prepared_at, wallet_reviewed_at,
                  terms_bound_at
        "#,
    )
    .bind(handoff_id)
    .bind(token_hash)
    .bind(&terms_hash)
    .bind(&creator_wallet)
    .bind(bound_at)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(crate::distribution_handoff_from_row)
        .transpose()?
        .ok_or_else(|| {
            DbError::DistributionAttributionConflict(
                "handoff, acquisition token, or immutable terms binding did not match".to_string(),
            )
        })
}

#[cfg(test)]
mod tests;
