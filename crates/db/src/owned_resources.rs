//! Atomic ownership for marketplace resources created by authenticated callers.
use crate::{
    auth::{self, AuthStoreError},
    security_writes::OwnedMutationError,
    Agent, Bounty, BountyStatus, Capability, DbError, DbResult, PostgresStore,
};
use sqlx::Row;
use uuid::Uuid;

pub(crate) fn agent_from_row(row: &sqlx::postgres::PgRow) -> DbResult<Agent> {
    Ok(Agent {
        id: row.try_get("id")?,
        handle: row.try_get("handle")?,
        status: crate::parse_agent_status(row.try_get("status")?)?,
        payout_wallet: row.try_get("payout_wallet")?,
        created_at: row.try_get("created_at")?,
    })
}

impl PostgresStore {
    /// A targeted lookup lets a second service instance use a newly registered agent.
    pub async fn get_agent(&self, id: Uuid) -> DbResult<Option<Agent>> {
        sqlx::query("SELECT id,handle,status,payout_wallet,created_at FROM agents WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| agent_from_row(&row))
            .transpose()
    }

    pub async fn create_owned_capability(
        &self,
        capability: &Capability,
        principal: &str,
    ) -> Result<Capability, OwnedMutationError> {
        let mut tx = self.pool.begin().await?;
        let permitted: bool = sqlx::query_scalar(
            "SELECT ($3='operator' OR EXISTS(SELECT 1 FROM security_resource_owners o WHERE o.resource_kind='agent' AND o.resource_id=$1 AND (o.principal_key=$3 OR EXISTS(SELECT 1 FROM security_resource_grants g WHERE g.resource_kind=o.resource_kind AND g.resource_id=o.resource_id AND g.grantee_key=$3 AND g.permission='write' AND g.expires_at>now())))) AND EXISTS(SELECT 1 FROM agents WHERE id=$2)",
        )
        .bind(capability.agent_id.to_string())
        .bind(capability.agent_id)
        .bind(principal)
        .fetch_one(&mut *tx)
        .await?;
        if !permitted {
            return Err(AuthStoreError::Conflict.into());
        }
        let inserted = sqlx::query(
            "INSERT INTO capabilities(id,agent_id,class,template_slugs,min_price,max_price,currency,latency_seconds,supported_verifiers) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT DO NOTHING",
        )
        .bind(capability.id)
        .bind(capability.agent_id)
        .bind(format!("{:?}", capability.class))
        .bind(serde_json::to_value(&capability.template_slugs)?)
        .bind(capability.min_price.amount)
        .bind(capability.max_price.amount)
        .bind(&capability.min_price.currency)
        .bind(i64::try_from(capability.latency_seconds).map_err(|_| {
            DbError::IntegerOverflow("capability.latency_seconds".into())
        })?)
        .bind(serde_json::to_value(&capability.supported_verifiers)?)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(OwnedMutationError::Conflict);
        }
        auth::claim_resource_ownership_in_transaction(
            &mut tx,
            "capability",
            &capability.id.to_string(),
            principal,
        )
        .await?;
        tx.commit().await?;
        Ok(capability.clone())
    }

    pub async fn create_owned_pooled_bounty(
        &self,
        bounty: &Bounty,
        principal: &str,
        key_hash: &str,
        payload_hash: &str,
        limit: i64,
    ) -> Result<Bounty, OwnedMutationError> {
        if bounty.status != BountyStatus::Unfunded || bounty.help_request_id.is_some() {
            return Err(AuthStoreError::InvalidInput.into());
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("pooled_publication:{principal}"))
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT payload_hash,result_json FROM security_mutation_results WHERE principal_key=$1 AND operation='pooled_publication' AND idempotency_hash=$2")
            .bind(principal).bind(key_hash).fetch_optional(&mut *tx).await? {
            if row.try_get::<String, _>("payload_hash")? != payload_hash {
                return Err(OwnedMutationError::Conflict);
            }
            let original: Bounty = serde_json::from_value(row.try_get("result_json")?)?;
            // Creation replies are immutable; the resource itself may since have advanced.
            let current = sqlx::query("SELECT * FROM bounties WHERE id=$1")
                .bind(original.id).fetch_one(&mut *tx).await?;
            return Ok(crate::bounty_from_row(&current)?);
        }
        auth::consume_security_quota_in_transaction(
            &mut tx,
            principal,
            "publication",
            key_hash,
            payload_hash,
            limit,
            86400,
        )
        .await?;
        let inserted = sqlx::query("INSERT INTO bounties(id,help_request_id,title,template_slug,amount,currency,funding_targets,funding_mode,privacy,status,terms_hash,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT DO NOTHING RETURNING *")
            .bind(bounty.id).bind(bounty.help_request_id).bind(&bounty.title).bind(&bounty.template_slug)
            .bind(bounty.amount.amount).bind(&bounty.amount.currency).bind(serde_json::to_value(&bounty.funding_targets)?)
            .bind(format!("{:?}",bounty.funding_mode)).bind(format!("{:?}",bounty.privacy)).bind(format!("{:?}",bounty.status))
            .bind(&bounty.terms_hash).bind(bounty.created_at).fetch_optional(&mut *tx).await?
            .ok_or(OwnedMutationError::Conflict)?;
        let saved = crate::bounty_from_row(&inserted)?;
        auth::claim_resource_ownership_in_transaction(
            &mut tx,
            "bounty",
            &bounty.id.to_string(),
            principal,
        )
        .await?;
        sqlx::query("INSERT INTO security_mutation_results(principal_key,operation,idempotency_hash,payload_hash,result_json) VALUES($1,'pooled_publication',$2,$3,$4)")
            .bind(principal).bind(key_hash).bind(payload_hash).bind(serde_json::to_value(&saved)?)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use domain::{CapabilityClass, FundingMode, Money, PrivacyLevel, VerifierKind};
    use sha2::{Digest, Sha256};

    fn hash(value: &str) -> String {
        hex::encode(Sha256::digest(value.as_bytes()))
    }

    async fn stores() -> (PostgresStore, PostgresStore) {
        let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").expect("isolated fixture DB");
        crate::test_support::assert_isolated_security_database(&url);
        let first = PostgresStore::connect(&url).await.unwrap();
        first.migrate().await.unwrap();
        (first, PostgresStore::connect(&url).await.unwrap())
    }

    fn bounty() -> Bounty {
        Bounty::new(
            "Local pooled fixture",
            "write-docs-for-area",
            Money::new(1000, "usdc").unwrap(),
            FundingMode::Simulated,
            PrivacyLevel::Private,
        )
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn capability_owner_and_record_commit_together_for_durable_agent() {
        let (first, second) = stores().await;
        let principal = format!("account:{}", Uuid::new_v4());
        let other = format!("account:{}", Uuid::new_v4());
        let agent = Agent::new(format!("local-capability-{}", Uuid::new_v4()));
        first
            .register_owned_agent(&agent, &principal, &hash("register"), &hash("agent"), 5)
            .await
            .unwrap();
        assert_eq!(
            second.get_agent(agent.id).await.unwrap().unwrap().id,
            agent.id
        );
        let mut capability = Capability {
            id: Uuid::new_v4(),
            agent_id: agent.id,
            class: CapabilityClass::Coding,
            template_slugs: vec!["write-docs-for-area".into()],
            min_price: Money::new(1, "usdc").unwrap(),
            max_price: Money::new(1000, "usdc").unwrap(),
            latency_seconds: 60,
            supported_verifiers: vec![VerifierKind::JsonSchema],
        };
        assert!(second
            .create_owned_capability(&capability, &other)
            .await
            .is_err());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM capabilities WHERE agent_id=$1")
            .bind(agent.id)
            .fetch_one(&first.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "knowing a durable agent UUID is insufficient");
        first
            .claim_resource_ownership("capability", &capability.id.to_string(), &other)
            .await
            .unwrap();
        assert!(second
            .create_owned_capability(&capability, &principal)
            .await
            .is_err());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM capabilities WHERE agent_id=$1")
            .bind(agent.id)
            .fetch_one(&first.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "ownership failure leaves no capability row");
        capability.id = Uuid::new_v4();
        second
            .create_owned_capability(&capability, &principal)
            .await
            .unwrap();
        assert!(first
            .resource_is_authorized(
                "capability",
                &capability.id.to_string(),
                &principal,
                "write"
            )
            .await
            .unwrap());
        assert!(!first
            .resource_is_authorized("capability", &capability.id.to_string(), &other, "read")
            .await
            .unwrap());
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn pooled_replay_shares_publication_quota_and_preserves_current_state() {
        let (first, second) = stores().await;
        let principal = format!("account:{}", Uuid::new_v4());
        for index in 0..2 {
            let key = hash(&format!("trial-{index}"));
            let trial = crate::NewTrialBounty {
                id: Uuid::new_v4(),
                idempotency_key: hash(&format!("{principal}:{key}")),
                request_fingerprint: key.clone(),
                title: "Local trial fixture".into(),
                goal: "Verify a shared publication limit".into(),
                acceptance_criteria: vec!["All paths share the same limit".into()],
                source_url: None,
                discovery_source: "security-test".into(),
                status: "open".into(),
                demo_agent_solution: serde_json::Value::Null,
                expires_at: Utc::now() + Duration::days(7),
            };
            first
                .create_owned_trial_bounty(&trial, &principal, &key, 3)
                .await
                .unwrap();
        }
        let a = bounty();
        let b = bounty();
        let key = hash("pooled:same-key");
        let payload = hash("same request");
        let (left, right) = tokio::join!(
            first.create_owned_pooled_bounty(&a, &principal, &key, &payload, 3),
            second.create_owned_pooled_bounty(&b, &principal, &key, &payload, 3)
        );
        let saved = left.unwrap();
        assert_eq!(saved.id, right.unwrap().id);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM security_quota_reservations WHERE principal_key=$1 AND action='publication'")
            .bind(&principal).fetch_one(&first.pool).await.unwrap();
        assert_eq!(count, 3, "unfunded and pooled publications share the quota");
        assert!(matches!(
            first
                .create_owned_pooled_bounty(&bounty(), &principal, &hash("fourth"), &payload, 3)
                .await,
            Err(OwnedMutationError::Auth(AuthStoreError::QuotaExceeded))
        ));
        assert!(matches!(
            first
                .create_owned_pooled_bounty(
                    &bounty(),
                    &principal,
                    &key,
                    &hash("changed content"),
                    3
                )
                .await,
            Err(OwnedMutationError::Conflict)
        ));
        sqlx::query("UPDATE bounties SET status='Claimable' WHERE id=$1")
            .bind(saved.id)
            .execute(&first.pool)
            .await
            .unwrap();
        let replay = second
            .create_owned_pooled_bounty(&a, &principal, &key, &payload, 3)
            .await
            .unwrap();
        assert_eq!(replay.id, saved.id);
        assert_eq!(
            replay.status,
            BountyStatus::Claimable,
            "creation retry cannot reset later state"
        );
        assert!(first
            .resource_is_authorized("bounty", &saved.id.to_string(), &principal, "write")
            .await
            .unwrap());

        let other = format!("account:{}", Uuid::new_v4());
        let conflicting = bounty();
        first
            .claim_resource_ownership("bounty", &conflicting.id.to_string(), &principal)
            .await
            .unwrap();
        assert!(second
            .create_owned_pooled_bounty(&conflicting, &other, &key, &payload, 3)
            .await
            .is_err());
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bounties WHERE id=$1)")
            .bind(conflicting.id)
            .fetch_one(&first.pool)
            .await
            .unwrap();
        let quota: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM security_quota_reservations WHERE principal_key=$1",
        )
        .bind(&other)
        .fetch_one(&first.pool)
        .await
        .unwrap();
        assert!(!exists);
        assert_eq!(
            quota, 0,
            "owner conflict rolls back pooled insertion and quota"
        );
    }
}
