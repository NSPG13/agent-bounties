//! Retention holds share the artifact lock with cleanup. Reasons are private.
use crate::{verification::EvidenceError, PostgresStore};
use serde_json::Value;
use uuid::Uuid;

impl PostgresStore {
    pub async fn open_artifact_dispute(
        &self,
        artifact: Uuid,
        principal: &str,
        reason: &str,
    ) -> Result<Value, EvidenceError> {
        if reason.trim().is_empty() || reason.len() > 2000 {
            return Err(EvidenceError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query_scalar::<_, Uuid>("SELECT a.id FROM verification_artifacts a WHERE a.id=$1 AND a.status IN ('uploading','ready') AND (a.owner_key=$2 OR EXISTS(SELECT 1 FROM security_resource_grants g WHERE g.resource_kind='verification_artifact' AND g.resource_id=a.id::text AND g.grantee_key=$2 AND g.permission='read' AND g.expires_at>now())) FOR UPDATE OF a")
            .bind(artifact).bind(principal).fetch_optional(&mut *tx).await?;
        if locked.is_none() {
            return Err(EvidenceError::NotFound);
        }
        sqlx::query("INSERT INTO verification_artifact_disputes(id,artifact_id,principal_key,reason) VALUES($1,$2,$3,$4) ON CONFLICT(artifact_id,principal_key) WHERE resolved_at IS NULL DO NOTHING")
            .bind(Uuid::new_v4()).bind(artifact).bind(principal).bind(reason).execute(&mut *tx).await?;
        let record: Value = sqlx::query_scalar("SELECT to_jsonb(d)-'principal_key' FROM verification_artifact_disputes d WHERE artifact_id=$1 AND principal_key=$2 AND resolved_at IS NULL")
            .bind(artifact).bind(principal).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE verification_artifacts SET dispute_hold=TRUE WHERE id=$1")
            .bind(artifact)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(record)
    }

    /// A participant can withdraw only their own dispute. Other holds remain.
    /// An expired read grant does not prevent that participant from withdrawing.
    pub async fn resolve_artifact_dispute(
        &self,
        id: Uuid,
        principal: &str,
        resolution: &str,
    ) -> Result<Value, EvidenceError> {
        if resolution.trim().is_empty() || resolution.len() > 2000 {
            return Err(EvidenceError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let artifact: Uuid = sqlx::query_scalar("SELECT a.id FROM verification_artifacts a JOIN verification_artifact_disputes d ON d.artifact_id=a.id WHERE d.id=$1 AND d.principal_key=$2 FOR UPDATE OF a")
            .bind(id).bind(principal).fetch_optional(&mut *tx).await?.ok_or(EvidenceError::NotFound)?;
        let existing: Option<String> =
            sqlx::query_scalar("SELECT resolution FROM verification_artifact_disputes WHERE id=$1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if existing.as_ref().is_some_and(|old| old != resolution) {
            return Err(EvidenceError::Conflict);
        }
        let record: Value = sqlx::query_scalar("UPDATE verification_artifact_disputes d SET resolved_at=COALESCE(resolved_at,now()),resolution=$2 WHERE id=$1 RETURNING to_jsonb(d)-'principal_key'")
            .bind(id).bind(resolution).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE verification_artifacts SET dispute_hold=EXISTS(SELECT 1 FROM verification_artifact_disputes WHERE artifact_id=$1 AND resolved_at IS NULL) WHERE id=$1")
            .bind(artifact).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broad_verification::Manifest;
    use std::collections::BTreeMap;

    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL"]
    async fn disputes_pending_work_and_winners_survive_cleanup() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        crate::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let owner = format!("account:{}", Uuid::new_v4());
        let reader = format!("account:{}", Uuid::new_v4());
        let manifest =
            Manifest::from_files(&BTreeMap::from([("work.txt".into(), b"original".to_vec())]))
                .unwrap();
        let mut ids = Vec::new();
        for key in ["disputed", "pending", "winner", "closed", "abandoned"] {
            let a = store
                .reserve_artifact(&owner, key, &manifest, 10 * 1024 * 1024 * 1024)
                .await
                .unwrap();
            store
                .finish_artifact(a.id, &owner, &a.manifest_hash)
                .await
                .unwrap();
            sqlx::query(
                "UPDATE verification_artifacts SET retain_until=now()-INTERVAL '1 day' WHERE id=$1",
            )
            .bind(a.id)
            .execute(&store.pool)
            .await
            .unwrap();
            ids.push(a.id);
        }
        let own = store
            .open_artifact_dispute(ids[0], &owner, "Check the saved version")
            .await
            .unwrap();
        assert!(matches!(
            store
                .open_artifact_dispute(ids[0], &reader, "No grant yet")
                .await,
            Err(EvidenceError::NotFound)
        ));
        store
            .grant_resource_access(
                "verification_artifact",
                &ids[0].to_string(),
                &owner,
                &reader,
                "read",
                chrono::Utc::now() + chrono::Duration::days(1),
            )
            .await
            .unwrap();
        let other = store
            .open_artifact_dispute(ids[0], &reader, "Another unresolved question")
            .await
            .unwrap();
        let retried = store
            .open_artifact_dispute(ids[0], &owner, "Retry does not replace the reason")
            .await
            .unwrap();
        assert_eq!(own, retried);
        let other_id = Uuid::parse_str(other["id"].as_str().unwrap()).unwrap();
        assert!(matches!(
            store
                .resolve_artifact_dispute(other_id, &owner, "Cannot close someone else's dispute")
                .await,
            Err(EvidenceError::NotFound)
        ));
        let own_id = Uuid::parse_str(own["id"].as_str().unwrap()).unwrap();
        store
            .resolve_artifact_dispute(own_id, &owner, "My question is resolved")
            .await
            .unwrap();
        assert!(
            store
                .owned_artifact(ids[0], &owner)
                .await
                .unwrap()
                .dispute_hold
        );
        store
            .prepare_artifact_scope(ids[1], &owner, &serde_json::json!({"round":1}))
            .await
            .unwrap();
        for id in [ids[2], ids[3]] {
            store
                .confirm_artifact(id, &owner, &serde_json::json!({"round":1}))
                .await
                .unwrap();
            store
                .retain_closed_artifact(
                    id,
                    chrono::Utc::now() - chrono::Duration::days(400),
                    id == ids[2],
                )
                .await
                .unwrap();
        }
        let deletable: Vec<_> = store
            .expired_artifacts()
            .await
            .unwrap()
            .iter()
            .map(|a| a.id)
            .collect();
        assert!(!deletable.contains(&ids[0]));
        assert!(!deletable.contains(&ids[1]));
        assert!(!deletable.contains(&ids[2]));
        assert!(deletable.contains(&ids[3]));
        assert!(deletable.contains(&ids[4]));
        assert!(store
            .open_artifact_dispute(ids[4], &owner, "Too late: deletion already started")
            .await
            .is_err());
        store
            .revoke_resource_access(
                "verification_artifact",
                &ids[0].to_string(),
                &owner,
                &reader,
            )
            .await
            .unwrap();
        store
            .resolve_artifact_dispute(other_id, &reader, "Resolved even after access expires")
            .await
            .unwrap();
        assert!(store
            .expired_artifacts()
            .await
            .unwrap()
            .iter()
            .any(|a| a.id == ids[0]));
    }
}
