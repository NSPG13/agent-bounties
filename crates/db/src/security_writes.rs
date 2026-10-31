use crate::{
    auth::{self, AuthStoreError},
    Agent, DbError, NewTrialBounty, PostgresStore, TrialBounty,
};
use sqlx::Row;

#[derive(Debug, thiserror::Error)]
pub enum OwnedMutationError {
    #[error(transparent)]
    Auth(#[from] AuthStoreError),
    #[error("durable mutation unavailable")]
    Store(#[from] sqlx::Error),
    #[error("durable mutation unavailable")]
    Db(#[from] DbError),
    #[error("invalid stored mutation")]
    Json(#[from] serde_json::Error),
    #[error("handle or idempotency key conflicts")]
    Conflict,
}

impl PostgresStore {
    pub async fn register_owned_agent(
        &self,
        agent: &Agent,
        principal: &str,
        key_hash: &str,
        payload_hash: &str,
        limit: i64,
    ) -> Result<Agent, OwnedMutationError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("registration:{principal}"))
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT payload_hash,result_json FROM security_mutation_results WHERE principal_key=$1 AND operation='registration' AND idempotency_hash=$2")
            .bind(principal).bind(key_hash).fetch_optional(&mut *tx).await? {
            if row.try_get::<String,_>("payload_hash")? != payload_hash { return Err(OwnedMutationError::Conflict); }
            let original: Agent = serde_json::from_value(row.try_get("result_json")?)?;
            let current = sqlx::query("SELECT id,handle,status,payout_wallet,created_at FROM agents WHERE id=$1")
                .bind(original.id).fetch_one(&mut *tx).await?;
            return Ok(crate::owned_resources::agent_from_row(&current)?);
        }
        auth::consume_security_quota_in_transaction(
            &mut tx,
            principal,
            "registration",
            key_hash,
            payload_hash,
            limit,
            86400,
        )
        .await?;
        let inserted = sqlx::query("INSERT INTO agents(id,handle,status,payout_wallet,created_at) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING RETURNING id,handle,status,payout_wallet,created_at")
            .bind(agent.id).bind(&agent.handle).bind(format!("{:?}",agent.status)).bind(&agent.payout_wallet).bind(agent.created_at).fetch_optional(&mut *tx).await?
            .ok_or(OwnedMutationError::Conflict)?;
        let saved = crate::owned_resources::agent_from_row(&inserted)?;
        auth::claim_resource_ownership_in_transaction(
            &mut tx,
            "agent",
            &agent.id.to_string(),
            principal,
        )
        .await?;
        sqlx::query("INSERT INTO security_mutation_results(principal_key,operation,idempotency_hash,payload_hash,result_json) VALUES($1,'registration',$2,$3,$4)")
            .bind(principal).bind(key_hash).bind(payload_hash).bind(serde_json::to_value(&saved)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(saved)
    }

    pub async fn create_owned_trial_bounty(
        &self,
        trial: &NewTrialBounty,
        principal: &str,
        key_hash: &str,
        limit: i64,
    ) -> Result<TrialBounty, OwnedMutationError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("publication:{principal}"))
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT payload_hash,result_json FROM security_mutation_results WHERE principal_key=$1 AND operation='publication' AND idempotency_hash=$2")
            .bind(principal).bind(key_hash).fetch_optional(&mut *tx).await? {
            if row.try_get::<String,_>("payload_hash")? != trial.request_fingerprint { return Err(OwnedMutationError::Conflict); }
            let original: TrialBounty = serde_json::from_value(row.try_get("result_json")?)?;
            let current = sqlx::query("SELECT * FROM trial_bounties WHERE id=$1")
                .bind(original.id).fetch_one(&mut *tx).await?;
            return Ok(crate::trial_bounty_from_row(current)?);
        }
        auth::consume_security_quota_in_transaction(
            &mut tx,
            principal,
            "publication",
            key_hash,
            &trial.request_fingerprint,
            limit,
            86400,
        )
        .await?;
        // The external key is already scoped and hashed by the API. It cannot collide with another caller.
        let row = sqlx::query("INSERT INTO trial_bounties(id,idempotency_key,request_fingerprint,title,goal,acceptance_criteria,source_url,discovery_source,status,demo_agent_solution,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING *")
            .bind(trial.id).bind(&trial.idempotency_key).bind(&trial.request_fingerprint).bind(&trial.title).bind(&trial.goal).bind(serde_json::to_value(&trial.acceptance_criteria)?).bind(&trial.source_url).bind(&trial.discovery_source).bind(&trial.status).bind(&trial.demo_agent_solution).bind(trial.expires_at).fetch_one(&mut *tx).await?;
        let saved = crate::trial_bounty_from_row(row)?;
        auth::claim_resource_ownership_in_transaction(
            &mut tx,
            "publication",
            &trial.id.to_string(),
            principal,
        )
        .await?;
        sqlx::query("INSERT INTO security_mutation_results(principal_key,operation,idempotency_hash,payload_hash,result_json) VALUES($1,'publication',$2,$3,$4)")
            .bind(principal).bind(key_hash).bind(&trial.request_fingerprint).bind(serde_json::to_value(&saved)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    fn hash(value: &str) -> String {
        hex::encode(Sha256::digest(value.as_bytes()))
    }

    async fn stores() -> (PostgresStore, PostgresStore) {
        let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL")
            .expect("isolated test database required");
        crate::test_support::assert_isolated_security_database(&url);
        let first = PostgresStore::connect(&url).await.unwrap();
        first.migrate().await.unwrap();
        let second = PostgresStore::connect(&url).await.unwrap();
        (first, second)
    }

    async fn counts(store: &PostgresStore, principal: &str, operation: &str) -> (i64, i64, i64) {
        let owners = sqlx::query_scalar(
            "SELECT count(*) FROM security_resource_owners WHERE principal_key=$1",
        )
        .bind(principal)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        let quotas = sqlx::query_scalar(
            "SELECT count(*) FROM security_quota_reservations WHERE principal_key=$1 AND action=$2",
        )
        .bind(principal)
        .bind(operation)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        let results = sqlx::query_scalar("SELECT count(*) FROM security_mutation_results WHERE principal_key=$1 AND operation=$2")
            .bind(principal).bind(operation).fetch_one(&store.pool).await.unwrap();
        (owners, quotas, results)
    }

    fn agent() -> Agent {
        let mut agent = Agent::new(format!("local-security-{}", Uuid::new_v4()));
        agent.payout_wallet = Some("0x1111111111111111111111111111111111111111".into());
        agent
    }

    fn trial(principal: &str, key: &str) -> NewTrialBounty {
        NewTrialBounty {
            id: Uuid::new_v4(),
            idempotency_key: hash(&format!("{principal}:{key}")),
            request_fingerprint: hash(key),
            title: "Synthetic local security test".into(),
            goal: "Confirm publication durability without running a model".into(),
            acceptance_criteria: vec!["Exactly one stored publication".into()],
            source_url: None,
            discovery_source: "security-test".into(),
            status: "open".into(),
            demo_agent_solution: serde_json::json!({"status":"pending","model_called":false}),
            expires_at: Utc::now() + Duration::days(7),
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn registration_replay_conflicts_and_ownership_are_atomic_across_pools() {
        let (first, second) = stores().await;
        let principal = format!("account:{}", Uuid::new_v4());
        let other = format!("account:{}", Uuid::new_v4());
        let initial = agent();
        let mut retry = initial.clone();
        retry.id = Uuid::new_v4();
        let key = hash("registration-replay");
        let payload = hash(&initial.handle);
        let (left, right) = tokio::join!(
            first.register_owned_agent(&initial, &principal, &key, &payload, 5),
            second.register_owned_agent(&retry, &principal, &key, &payload, 5)
        );
        let saved = left.unwrap();
        assert_eq!(saved.id, right.unwrap().id);
        assert_eq!(counts(&first, &principal, "registration").await, (1, 1, 1));
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM agents WHERE handle=$1")
            .bind(&initial.handle)
            .fetch_one(&first.pool)
            .await
            .unwrap();
        assert_eq!(rows, 1, "two pools commit only one agent");

        let mut duplicate = initial.clone();
        duplicate.id = Uuid::new_v4();
        assert!(matches!(
            second
                .register_owned_agent(&duplicate, &other, &key, &payload, 5)
                .await,
            Err(OwnedMutationError::Conflict)
        ));
        assert_eq!(counts(&first, &other, "registration").await, (0, 0, 0));
        let ghost: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agents WHERE id=$1)")
            .bind(duplicate.id)
            .fetch_one(&first.pool)
            .await
            .unwrap();
        assert!(
            !ghost,
            "duplicate handle leaves no agent, owner, quota or replay result"
        );
        assert!(matches!(
            first
                .register_owned_agent(&agent(), &principal, &key, &hash("changed request"), 5)
                .await,
            Err(OwnedMutationError::Conflict)
        ));
        assert_eq!(counts(&first, &principal, "registration").await, (1, 1, 1));

        let independent = agent();
        let independent = second
            .register_owned_agent(&independent, &other, &key, &payload, 5)
            .await
            .unwrap();
        assert_ne!(
            saved.id, independent.id,
            "same external key is scoped to each principal"
        );
        assert!(first
            .resource_is_authorized("agent", &saved.id.to_string(), &principal, "write")
            .await
            .unwrap());
        assert!(!first
            .resource_is_authorized("agent", &saved.id.to_string(), &other, "read")
            .await
            .unwrap());

        let uncommitted = agent();
        first
            .claim_resource_ownership("agent", &uncommitted.id.to_string(), &other)
            .await
            .unwrap();
        assert!(matches!(
            first
                .register_owned_agent(
                    &uncommitted,
                    &principal,
                    &hash("owner-conflict"),
                    &payload,
                    5
                )
                .await,
            Err(OwnedMutationError::Auth(AuthStoreError::Conflict))
        ));
        let ghost: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agents WHERE id=$1)")
            .bind(uncommitted.id)
            .fetch_one(&first.pool)
            .await
            .unwrap();
        assert!(!ghost, "ownership failure rolls back the preceding insert");
        assert_eq!(counts(&first, &principal, "registration").await, (1, 1, 1));
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn registration_daily_quota_is_shared_across_pools_and_retries() {
        let (first, second) = stores().await;
        let principal = format!("account:{}", Uuid::new_v4());
        let mut tasks = Vec::new();
        for index in 0..10 {
            let store = if index % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            let principal = principal.clone();
            tasks.push(tokio::spawn(async move {
                let agent = agent();
                let key = hash(&format!("concurrent-{index}"));
                let payload = hash(&agent.handle);
                let result = store
                    .register_owned_agent(&agent, &principal, &key, &payload, 5)
                    .await;
                (agent, key, payload, result)
            }));
        }
        let mut committed = Vec::new();
        let mut refused = 0;
        for task in tasks {
            let (agent, key, payload, result) = task.await.unwrap();
            match result {
                Ok(saved) => committed.push((saved, key, payload)),
                Err(OwnedMutationError::Auth(AuthStoreError::QuotaExceeded)) => {
                    refused += 1;
                    let exists: bool =
                        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agents WHERE id=$1)")
                            .bind(agent.id)
                            .fetch_one(&first.pool)
                            .await
                            .unwrap();
                    assert!(!exists);
                }
                result => panic!("unexpected registration result: {result:?}"),
            }
        }
        assert_eq!((committed.len(), refused), (5, 5));
        assert_eq!(counts(&first, &principal, "registration").await, (5, 5, 5));
        let (saved, key, payload) = &committed[0];
        let replay = second
            .register_owned_agent(saved, &principal, key, payload, 5)
            .await
            .unwrap();
        assert_eq!(
            replay.id, saved.id,
            "a retry succeeds even when today's quota is full"
        );
        assert_eq!(counts(&first, &principal, "registration").await, (5, 5, 5));
        sqlx::query("UPDATE security_quota_reservations SET created_at=now()-interval '25 hours' WHERE principal_key=$1 AND action='registration'")
            .bind(&principal).execute(&first.pool).await.unwrap();
        first
            .register_owned_agent(
                &agent(),
                &principal,
                &hash("next-day"),
                &hash("next-day-payload"),
                5,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn publication_replay_is_scoped_atomic_and_does_not_replace_demo() {
        let (first, second) = stores().await;
        let principal = format!("account:{}", Uuid::new_v4());
        let other = format!("wallet:8453:{}", Uuid::new_v4());
        let original = trial(&principal, "publish-replay");
        let mut retry = original.clone();
        retry.id = Uuid::new_v4();
        let key = hash("publish-replay");
        let (left, right) = tokio::join!(
            first.create_owned_trial_bounty(&original, &principal, &key, 3),
            second.create_owned_trial_bounty(&retry, &principal, &key, 3)
        );
        let saved = left.unwrap();
        assert_eq!(saved, right.unwrap());
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM trial_bounties WHERE idempotency_key=$1")
                .bind(&original.idempotency_key)
                .fetch_one(&first.pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
        assert_eq!(counts(&first, &principal, "publication").await, (1, 1, 1));
        retry.demo_agent_solution = serde_json::json!({"status":"unexpected second demo"});
        let replay = second
            .create_owned_trial_bounty(&retry, &principal, &key, 3)
            .await
            .unwrap();
        assert_eq!(
            replay, saved,
            "retry returns the first result without replacing or duplicating its demo"
        );
        let stored_demo: serde_json::Value =
            sqlx::query_scalar("SELECT demo_agent_solution FROM trial_bounties WHERE id=$1")
                .bind(saved.id)
                .fetch_one(&first.pool)
                .await
                .unwrap();
        assert_eq!(stored_demo, original.demo_agent_solution);
        retry.request_fingerprint = hash("different publication payload");
        assert!(matches!(
            second
                .create_owned_trial_bounty(&retry, &principal, &key, 3)
                .await,
            Err(OwnedMutationError::Conflict)
        ));
        let independent = second
            .create_owned_trial_bounty(&trial(&other, "publish-replay"), &other, &key, 3)
            .await
            .unwrap();
        assert_ne!(independent.id, saved.id);
        assert!(!first
            .resource_is_authorized("publication", &saved.id.to_string(), &other, "read")
            .await
            .unwrap());

        for index in 1..3 {
            let key = format!("publish-{index}");
            first
                .create_owned_trial_bounty(&trial(&principal, &key), &principal, &hash(&key), 3)
                .await
                .unwrap();
        }
        let rejected = trial(&principal, "publish-fourth");
        assert!(matches!(
            second
                .create_owned_trial_bounty(&rejected, &principal, &hash("publish-fourth"), 3)
                .await,
            Err(OwnedMutationError::Auth(AuthStoreError::QuotaExceeded))
        ));
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trial_bounties WHERE id=$1)")
                .bind(rejected.id)
                .fetch_one(&first.pool)
                .await
                .unwrap();
        assert!(!exists);
        assert_eq!(counts(&first, &principal, "publication").await, (3, 3, 3));
        assert_eq!(
            second
                .create_owned_trial_bounty(&original, &principal, &key, 3)
                .await
                .unwrap(),
            saved
        );
        sqlx::query("UPDATE security_quota_reservations SET created_at=now()-interval '25 hours' WHERE principal_key=$1 AND action='publication'")
            .bind(&principal).execute(&first.pool).await.unwrap();
        first
            .create_owned_trial_bounty(&rejected, &principal, &hash("publish-fourth"), 3)
            .await
            .unwrap();

        let conflicting = trial(&principal, "publication-owner-conflict");
        first
            .claim_resource_ownership("publication", &conflicting.id.to_string(), &other)
            .await
            .unwrap();
        let before = counts(&first, &principal, "publication").await;
        assert!(matches!(
            first
                .create_owned_trial_bounty(
                    &conflicting,
                    &principal,
                    &hash("publication-owner-conflict"),
                    3
                )
                .await,
            Err(OwnedMutationError::Auth(AuthStoreError::Conflict))
        ));
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trial_bounties WHERE id=$1)")
                .bind(conflicting.id)
                .fetch_one(&first.pool)
                .await
                .unwrap();
        assert!(
            !exists,
            "ownership failure rolls back a publication and its quota"
        );
        assert_eq!(counts(&first, &principal, "publication").await, before);
    }
}
