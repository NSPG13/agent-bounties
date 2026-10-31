//! Durable service checkpoints. They never authorize signatures or payments.
use super::PostgresStore;
use chrono::{DateTime, Utc};
use sqlx::Row;

#[derive(Debug, Clone)]
pub struct VerifierCheckpoint {
    pub component: String,
    pub issued_at: DateTime<Utc>,
    pub healthy: bool,
}

impl PostgresStore {
    pub async fn record_verifier_checkpoint(
        &self,
        release: &str,
        checkpoint: &VerifierCheckpoint,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO verifier_runtime_checkpoints (release_id,component,issued_at,healthy) VALUES ($1,$2,$3,$4)
             ON CONFLICT (release_id,component) DO UPDATE SET issued_at=EXCLUDED.issued_at,healthy=EXCLUDED.healthy,received_at=now()
             WHERE verifier_runtime_checkpoints.issued_at < EXCLUDED.issued_at",
        )
        .bind(release).bind(&checkpoint.component).bind(checkpoint.issued_at).bind(checkpoint.healthy)
        .execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn verifier_checkpoints(
        &self,
        release: &str,
    ) -> Result<Vec<VerifierCheckpoint>, sqlx::Error> {
        let rows = sqlx::query("SELECT component,issued_at,healthy FROM verifier_runtime_checkpoints WHERE release_id=$1")
            .bind(release).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(VerifierCheckpoint {
                    component: row.try_get("component")?,
                    issued_at: row.try_get("issued_at")?,
                    healthy: row.try_get("healthy")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn concurrent_replays_cannot_extend_health_or_erase_a_newer_failure() {
        let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
        crate::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let release = format!(
            "sha256:{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let checkpoint = VerifierCheckpoint {
            component: "runner".into(),
            issued_at: Utc::now(),
            healthy: true,
        };
        let (a, b) = tokio::join!(
            store.record_verifier_checkpoint(&release, &checkpoint),
            store.record_verifier_checkpoint(&release, &checkpoint)
        );
        assert_ne!(a.unwrap(), b.unwrap());
        let failed = VerifierCheckpoint {
            issued_at: checkpoint.issued_at + chrono::Duration::seconds(1),
            healthy: false,
            ..checkpoint.clone()
        };
        assert!(store
            .record_verifier_checkpoint(&release, &failed)
            .await
            .unwrap());
        assert!(!store
            .record_verifier_checkpoint(&release, &checkpoint)
            .await
            .unwrap());
        let other = PostgresStore::connect(&url).await.unwrap();
        let read = other.verifier_checkpoints(&release).await.unwrap();
        assert_eq!(read.len(), 1);
        assert!(!read[0].healthy);
        assert_eq!(
            read[0].issued_at.timestamp_micros(),
            failed.issued_at.timestamp_micros()
        );
        assert!(other
            .verifier_checkpoints(&format!("sha256:{}", "0".repeat(64)))
            .await
            .unwrap()
            .is_empty());
    }
}
