//! Durable immutable evidence, principal-scoped idempotency and capacity reservations.
use super::PostgresStore;
use broad_verification::{value_digest, Manifest, Plan, Report, MAX_UPLOAD};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error("evidence_unavailable")]
    Unavailable,
    #[error("evidence_not_found")]
    NotFound,
    #[error("evidence_conflict")]
    Conflict,
    #[error("evidence_capacity_exceeded")]
    Capacity,
    #[error("invalid_evidence")]
    Invalid,
}
impl From<sqlx::Error> for EvidenceError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
type Result<T> = std::result::Result<T, EvidenceError>;

#[derive(Clone, Debug, Serialize)]
pub struct Artifact {
    pub id: Uuid,
    #[serde(skip)]
    pub owner_key: String,
    pub manifest_hash: String,
    pub manifest: Manifest,
    pub size_bytes: i64,
    pub status: String,
    pub source: Option<Value>,
    pub canonical_scope: Option<Value>,
    #[serde(skip)]
    pub pending_scope: Option<Value>,
    pub winning: bool,
    pub dispute_hold: bool,
    pub retain_until: DateTime<Utc>,
}
fn artifact(row: sqlx::postgres::PgRow) -> Result<Artifact> {
    Ok(Artifact {
        id: row.try_get("id")?,
        owner_key: row.try_get("owner_key")?,
        manifest_hash: row.try_get("manifest_hash")?,
        manifest: serde_json::from_value(row.try_get("manifest")?)
            .map_err(|_| EvidenceError::Unavailable)?,
        size_bytes: row.try_get("size_bytes")?,
        status: row.try_get("status")?,
        source: row.try_get("source")?,
        canonical_scope: row.try_get("canonical_scope")?,
        pending_scope: row.try_get("pending_scope")?,
        winning: row.try_get("winning")?,
        dispute_hold: row.try_get("dispute_hold")?,
        retain_until: row.try_get("retain_until")?,
    })
}
#[derive(Debug, Serialize)]
pub struct VerificationRun {
    pub id: Uuid,
    pub artifact_id: Uuid,
    pub plan_hash: String,
    pub status: String,
    pub report: Option<Value>,
}
fn run_row(row: sqlx::postgres::PgRow) -> Result<VerificationRun> {
    Ok(VerificationRun {
        id: row.try_get("id")?,
        artifact_id: row.try_get("artifact_id")?,
        plan_hash: row.try_get("plan_hash")?,
        status: row.try_get("status")?,
        report: row.try_get("report")?,
    })
}

impl PostgresStore {
    pub async fn reserve_artifact(
        &self,
        owner: &str,
        key: &str,
        manifest: &Manifest,
        capacity: i64,
    ) -> Result<Artifact> {
        self.reserve_artifact_from_source(owner, key, manifest, capacity, None)
            .await
    }
    pub async fn artifact_by_request(
        &self,
        owner: &str,
        key: &str,
        source: &Value,
    ) -> Result<Option<Artifact>> {
        let row = sqlx::query(
            "SELECT * FROM verification_artifacts WHERE owner_key=$1 AND request_key=$2",
        )
        .bind(owner)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let item = artifact(row)?;
        if item.source.as_ref() != Some(source)
            || matches!(item.status.as_str(), "deleting" | "deleted")
        {
            return Err(EvidenceError::Conflict);
        }
        Ok(Some(item))
    }
    pub async fn reserve_artifact_from_source(
        &self,
        owner: &str,
        key: &str,
        manifest: &Manifest,
        capacity: i64,
        source: Option<&Value>,
    ) -> Result<Artifact> {
        manifest.validate().map_err(|_| EvidenceError::Invalid)?;
        if owner.is_empty()
            || owner.len() > 256
            || key.is_empty()
            || key.len() > 128
            || capacity < MAX_UPLOAD as i64
        {
            return Err(EvidenceError::Invalid);
        }
        let hash = value_digest(manifest).map_err(|_| EvidenceError::Invalid)?;
        let size = manifest.files.iter().map(|f| f.size as i64).sum::<i64>();
        let mut tx = self.pool.begin().await?;
        // Capacity and idempotency are shared across service instances.
        sqlx::query("SELECT pg_advisory_xact_lock(4270265042)")
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query(
            "SELECT * FROM verification_artifacts WHERE owner_key=$1 AND request_key=$2",
        )
        .bind(owner)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?
        {
            let old = artifact(row)?;
            if old.source.as_ref() != source
                || old.manifest_hash != hash
                || old.manifest != *manifest
                || matches!(old.status.as_str(), "deleting" | "deleted")
            {
                return Err(EvidenceError::Conflict);
            }
            tx.commit().await?;
            return Ok(old);
        }
        let used:i64=sqlx::query_scalar("SELECT COALESCE(sum(size_bytes),0)::bigint FROM verification_artifacts WHERE status <> 'deleted'").fetch_one(&mut *tx).await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM verification_artifacts WHERE owner_key=$1 AND created_at>now()-INTERVAL '1 day'").bind(owner).fetch_one(&mut *tx).await?;
        let daily:i64=sqlx::query_scalar("SELECT COALESCE(sum(size_bytes),0)::bigint FROM verification_artifacts WHERE owner_key=$1 AND created_at>now()-INTERVAL '1 day'").bind(owner).fetch_one(&mut *tx).await?;
        if count >= 30
            || used.checked_add(size).is_none_or(|n| n > capacity)
            || daily
                .checked_add(size)
                .is_none_or(|n| n > 1024 * 1024 * 1024)
        {
            return Err(EvidenceError::Capacity);
        }
        let row=sqlx::query("INSERT INTO verification_artifacts(id,owner_key,request_key,manifest_hash,manifest,size_bytes,source) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING *")
            .bind(Uuid::new_v4()).bind(owner).bind(key).bind(hash).bind(serde_json::to_value(manifest).map_err(|_|EvidenceError::Invalid)?).bind(size).bind(source).fetch_one(&mut *tx).await?;
        let id: Uuid = row.try_get("id")?;
        crate::auth::claim_resource_ownership_in_transaction(
            &mut tx,
            "verification_artifact",
            &id.to_string(),
            owner,
        )
        .await
        .map_err(|_| EvidenceError::Unavailable)?;
        tx.commit().await?;
        artifact(row)
    }
    pub async fn owned_artifact(&self, id: Uuid, owner: &str) -> Result<Artifact> {
        artifact(sqlx::query("SELECT * FROM verification_artifacts WHERE id=$1 AND owner_key=$2 AND status NOT IN ('deleting','deleted')").bind(id).bind(owner).fetch_optional(&self.pool).await?.ok_or(EvidenceError::NotFound)?)
    }
    /// Caller has verified every uploaded object, including readback of exact bytes.
    pub async fn finish_artifact(
        &self,
        id: Uuid,
        owner: &str,
        manifest_hash: &str,
    ) -> Result<Artifact> {
        artifact(sqlx::query("UPDATE verification_artifacts SET status='ready',updated_at=now() WHERE id=$1 AND owner_key=$2 AND manifest_hash=$3 AND status IN ('uploading','ready') RETURNING *").bind(id).bind(owner).bind(manifest_hash).fetch_optional(&self.pool).await?.ok_or(EvidenceError::Conflict)?)
    }
    pub async fn begin_verification_run(
        &self,
        owner: &str,
        artifact_id: Uuid,
        plan: &Plan,
    ) -> Result<VerificationRun> {
        broad_verification::validate_plan(plan).map_err(|_| EvidenceError::Invalid)?;
        let hash = value_digest(plan).map_err(|_| EvidenceError::Invalid)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,4270265043))")
            .bind(owner)
            .execute(&mut *tx)
            .await?;
        if let Some(row)=sqlx::query("SELECT * FROM verification_runs WHERE owner_key=$1 AND artifact_id=$2 AND plan_hash=$3").bind(owner).bind(artifact_id).bind(&hash).fetch_optional(&mut *tx).await? {tx.commit().await?;return run_row(row);}
        let recent:i64=sqlx::query_scalar("SELECT count(*) FROM verification_runs WHERE owner_key=$1 AND created_at>now()-INTERVAL '1 day'").bind(owner).fetch_one(&mut *tx).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(4270265044)")
            .execute(&mut *tx)
            .await?;
        let pending:i64=sqlx::query_scalar("SELECT count(*) FROM verification_runs WHERE status IN ('queued','running','unavailable') AND attempts<3").fetch_one(&mut *tx).await?;
        if recent >= 30 || pending >= 1000 {
            return Err(EvidenceError::Capacity);
        }
        let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM verification_artifacts WHERE id=$1 AND owner_key=$2 AND status='ready')").bind(artifact_id).bind(owner).fetch_one(&mut *tx).await?;
        if !ready {
            return Err(EvidenceError::NotFound);
        }
        sqlx::query("INSERT INTO verification_plans(owner_key,plan_hash,document) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(owner).bind(&hash).bind(serde_json::to_value(plan).map_err(|_|EvidenceError::Invalid)?).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO verification_runs(id,owner_key,artifact_id,plan_hash) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(Uuid::new_v4()).bind(owner).bind(artifact_id).bind(&hash).execute(&mut *tx).await?;
        let row=sqlx::query("SELECT * FROM verification_runs WHERE owner_key=$1 AND artifact_id=$2 AND plan_hash=$3").bind(owner).bind(artifact_id).bind(hash).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        run_row(row)
    }
    pub async fn finish_verification_run(
        &self,
        id: Uuid,
        owner: &str,
        lease: Uuid,
        report: &Report,
    ) -> Result<VerificationRun> {
        let mut tx = self.pool.begin().await?;
        let input=sqlx::query("SELECT a.manifest_hash,p.document FROM verification_runs r JOIN verification_artifacts a ON a.id=r.artifact_id JOIN verification_plans p ON p.owner_key=r.owner_key AND p.plan_hash=r.plan_hash WHERE r.id=$1 AND r.owner_key=$2 AND r.lease_token=$3 AND r.status='running' AND r.lease_expires_at>now() AND r.plan_hash=$4 FOR UPDATE OF r").bind(id).bind(owner).bind(lease).bind(&report.plan_hash).fetch_optional(&mut *tx).await?.ok_or(EvidenceError::Conflict)?;
        let plan: Plan = serde_json::from_value(input.try_get("document")?)
            .map_err(|_| EvidenceError::Unavailable)?;
        let questions: Vec<_> = plan
            .criteria
            .iter()
            .filter(|c| c.review.is_some())
            .collect();
        if report.schema_version != broad_verification::VERSION
            || report.payment_authorized
            || report.manifest_hash != input.try_get::<String, _>("manifest_hash")?
            || report.checks.len() != plan.checks.len()
            || report
                .checks
                .iter()
                .zip(&plan.checks)
                .any(|(r, c)| r.id != c.id || r.purpose != c.purpose || r.reason.len() > 4096)
            || serde_json::to_value(&questions).map_err(|_| EvidenceError::Invalid)?
                != serde_json::to_value(&report.review_questions)
                    .map_err(|_| EvidenceError::Invalid)?
            || (report.required_checks_passed
                && report.checks.iter().any(|r| {
                    r.purpose == broad_verification::Purpose::Required
                        && r.status != broad_verification::Status::Pass
                }))
        {
            return Err(EvidenceError::Invalid);
        }
        let row=sqlx::query("UPDATE verification_runs SET status=$2,report=$3,completed_at=now(),lease_token=NULL,lease_expires_at=NULL,next_attempt_at=CASE WHEN $4 THEN now() ELSE now()+INTERVAL '5 minutes' END WHERE id=$1 RETURNING *")
            .bind(id).bind(if report.checks.iter().any(|c|c.status==broad_verification::Status::CheckUnavailable) {"unavailable"}else{"completed"}).bind(serde_json::to_value(report).map_err(|_|EvidenceError::Invalid)?).bind(report.checks.iter().any(|c|c.status==broad_verification::Status::CheckUnavailable && c.reason=="requires_precommitted_bounded_runner")).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        run_row(row)
    }
    pub async fn verification_run(&self, id: Uuid, owner: &str) -> Result<VerificationRun> {
        run_row(
            sqlx::query("SELECT r.* FROM verification_runs r JOIN verification_artifacts a ON a.id=r.artifact_id WHERE r.id=$1 AND a.status NOT IN ('deleting','deleted') AND (r.owner_key=$2 OR EXISTS(SELECT 1 FROM security_resource_grants g WHERE g.resource_kind='verification_artifact' AND g.resource_id=a.id::text AND g.grantee_key=$2 AND g.permission='read' AND g.expires_at>now()))")
                .bind(id)
                .bind(owner)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(EvidenceError::NotFound)?,
        )
    }
}

pub struct VerificationJob {
    pub id: Uuid,
    pub owner: String,
    pub lease: Uuid,
    pub artifact: Artifact,
    pub plan: Plan,
}
impl PostgresStore {
    pub async fn take_verification_job(
        &self,
        runner_enabled: bool,
    ) -> Result<Option<VerificationJob>> {
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("SELECT r.id,r.owner_key,p.document FROM verification_runs r JOIN verification_plans p USING(owner_key,plan_hash) WHERE (r.status IN ('queued','unavailable') OR (r.status='running' AND r.lease_expires_at<now())) AND r.next_attempt_at<=now() AND r.attempts<3 AND (r.attempts=0 OR $1 OR NOT jsonb_path_exists(p.document, '$.checks[*].rule ? (@.checker == \"published_tests_v1\")')) ORDER BY r.created_at LIMIT 1 FOR UPDATE OF r SKIP LOCKED").bind(runner_enabled).fetch_optional(&mut *tx).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let id: Uuid = row.try_get("id")?;
        let owner: String = row.try_get("owner_key")?;
        let plan: Plan = serde_json::from_value(row.try_get("document")?)
            .map_err(|_| EvidenceError::Unavailable)?;
        let lease = Uuid::new_v4();
        let artifact_id:Uuid=sqlx::query_scalar("UPDATE verification_runs SET status='running',attempts=attempts+1,lease_token=$2,lease_expires_at=now()+INTERVAL '15 minutes' WHERE id=$1 RETURNING artifact_id").bind(id).bind(lease).fetch_one(&mut *tx).await?;
        let artifact=artifact(sqlx::query("SELECT * FROM verification_artifacts WHERE id=$1 AND owner_key=$2 AND status='ready' FOR SHARE").bind(artifact_id).bind(&owner).fetch_optional(&mut *tx).await?.ok_or(EvidenceError::NotFound)?)?;
        tx.commit().await?;
        Ok(Some(VerificationJob {
            id,
            owner,
            lease,
            artifact,
            plan,
        }))
    }
    pub async fn unavailable_verification_run(&self, id: Uuid, lease: Uuid) -> Result<()> {
        sqlx::query("UPDATE verification_runs SET status='unavailable',lease_token=NULL,lease_expires_at=NULL,next_attempt_at=now()+INTERVAL '5 minutes' WHERE id=$1 AND lease_token=$2 AND status='running'").bind(id).bind(lease).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broad_verification::{Check, Criterion, Purpose, Rule, VERSION};
    use std::collections::BTreeMap;
    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL"]
    async fn concurrent_artifacts_and_runs_survive_restarts_without_cross_owner_access() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        crate::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let other = PostgresStore::connect(&url).await.unwrap();
        let owner = format!("account:{}", Uuid::new_v4());
        let files = BTreeMap::from([("work.txt".into(), b"actual work".to_vec())]);
        let manifest = Manifest::from_files(&files).unwrap();
        let (a, b) = tokio::join!(
            store.reserve_artifact(&owner, "upload", &manifest, 10 * 1024 * 1024 * 1024),
            other.reserve_artifact(&owner, "upload", &manifest, 10 * 1024 * 1024 * 1024)
        );
        let a = a.unwrap();
        assert_eq!(a.id, b.unwrap().id);
        assert!(matches!(
            store.owned_artifact(a.id, "wrong-owner").await,
            Err(EvidenceError::NotFound)
        ));
        let changed = Manifest::from_files(&BTreeMap::from([("changed".into(), vec![1])])).unwrap();
        assert!(matches!(
            store
                .reserve_artifact(&owner, "upload", &changed, 10 * 1024 * 1024 * 1024)
                .await,
            Err(EvidenceError::Conflict)
        ));
        let plan = Plan {
            schema_version: VERSION.into(),
            time_windows: None,
            criteria: vec![Criterion {
                id: "delivery".into(),
                text: "Deliver work".into(),
                review: None,
            }],
            checks: vec![Check {
                id: "hash".into(),
                criteria: vec!["delivery".into()],
                purpose: Purpose::Required,
                rule: Rule::IntegrityV1,
            }],
        };
        assert!(store
            .begin_verification_run(&owner, a.id, &plan)
            .await
            .is_err());
        store
            .finish_artifact(a.id, &owner, &a.manifest_hash)
            .await
            .unwrap();
        let (r1, r2) = tokio::join!(
            store.begin_verification_run(&owner, a.id, &plan),
            other.begin_verification_run(&owner, a.id, &plan)
        );
        let r = r1.unwrap();
        assert_eq!(r.id, r2.unwrap().id);
        let job = store.take_verification_job(false).await.unwrap().unwrap();
        assert_eq!(job.id, r.id);
        assert!(other.take_verification_job(false).await.unwrap().is_none());
        let report = broad_verification::run(&plan, &manifest, &files).unwrap();
        assert!(store
            .finish_verification_run(r.id, &owner, Uuid::new_v4(), &report)
            .await
            .is_err());
        let mut forged = report.clone();
        forged.manifest_hash = broad_verification::digest(b"different files");
        assert!(matches!(
            store
                .finish_verification_run(r.id, &owner, job.lease, &forged)
                .await,
            Err(EvidenceError::Invalid)
        ));
        forged = report.clone();
        forged.checks.clear();
        assert!(matches!(
            store
                .finish_verification_run(r.id, &owner, job.lease, &forged)
                .await,
            Err(EvidenceError::Invalid)
        ));
        sqlx::query(
            "UPDATE verification_runs SET lease_expires_at=now()-INTERVAL '1 second' WHERE id=$1",
        )
        .bind(r.id)
        .execute(&store.pool)
        .await
        .unwrap();
        assert!(matches!(
            store
                .finish_verification_run(r.id, &owner, job.lease, &report)
                .await,
            Err(EvidenceError::Conflict)
        ));
        let resumed = other.take_verification_job(false).await.unwrap().unwrap();
        assert_eq!(resumed.id, r.id);
        assert_ne!(resumed.lease, job.lease);
        let finished = store
            .finish_verification_run(r.id, &owner, resumed.lease, &report)
            .await
            .unwrap();
        assert_eq!(finished.status, "completed");
        assert_eq!(
            other
                .begin_verification_run(&owner, a.id, &plan)
                .await
                .unwrap()
                .report
                .unwrap()["required_checks_passed"],
            true
        );
        assert!(matches!(
            other.verification_run(r.id, "wrong-owner").await,
            Err(EvidenceError::NotFound)
        ));
        assert!(store
            .finish_verification_run(r.id, &owner, job.lease, &report)
            .await
            .is_err());
    }
    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL"]
    async fn reviews_preserve_drafts_signatures_corrections_and_reviewer_handoff() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        crate::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let agent = format!("account:{}", Uuid::new_v4());
        let reviewer = format!("0x{}00000000", Uuid::new_v4().simple());
        let wallet = format!("wallet:8453:{reviewer}");
        let request = serde_json::json!({"bounty_contract":format!("0x{}00000000",Uuid::new_v4().simple()),"verifier":reviewer,"round":1,"response_hash":"0xoriginal","deadline":1});
        let assessment =
            serde_json::json!({"checks":[{"reason":"Exact original explanation","passed":false}]});
        let draft = store
            .save_review_report(&agent, "base-mainnet", &request, &assessment, None)
            .await
            .unwrap();
        let id = Uuid::parse_str(draft["id"].as_str().unwrap()).unwrap();
        assert_eq!(
            store
                .save_review_report(&agent, "base-mainnet", &request, &assessment, None)
                .await
                .unwrap()["id"],
            draft["id"]
        );
        assert!(matches!(
            store.review_report(id, "unrelated").await,
            Err(EvidenceError::NotFound)
        ));
        assert_eq!(
            store.review_report(id, &wallet).await.unwrap()["assessment"],
            assessment
        );
        assert!(store
            .review_report_phase(
                id,
                &wallet,
                "confirmed",
                &serde_json::json!({"canonical_event_id":"unproven"})
            )
            .await
            .is_err());
        let signature = serde_json::json!({"signature":"verified-by-API","passed":false});
        assert_eq!(
            store
                .review_report_phase(id, &wallet, "signed", &signature)
                .await
                .unwrap()["status"],
            "signed"
        );
        assert!(store
            .review_report_phase(
                id,
                &agent,
                "signed",
                &serde_json::json!({"signature":"different"})
            )
            .await
            .is_err());
        let tx = serde_json::json!({"transaction_hash":"0xtx"});
        assert_eq!(
            store
                .review_report_phase(id, &wallet, "submitted", &tx)
                .await
                .unwrap()["status"],
            "submitted"
        );
        let event =
            serde_json::json!({"transaction_hash":"0xtx","canonical_event_id":"indexed-event"});
        assert_eq!(
            store
                .review_report_phase(id, &wallet, "confirmed", &event)
                .await
                .unwrap()["status"],
            "confirmed"
        );
        assert_eq!(
            store
                .review_report_phase(id, &agent, "confirmed", &event)
                .await
                .unwrap()["status"],
            "confirmed"
        );
        let mut correction = request.clone();
        correction["response_hash"] = serde_json::json!("0xcorrection");
        let revised = store
            .save_review_report(
                &agent,
                "base-mainnet",
                &correction,
                &serde_json::json!({"checks":[]}),
                Some(id),
            )
            .await
            .unwrap();
        assert_ne!(revised["id"], draft["id"]);
        assert_eq!(revised["previous_report_id"], draft["id"]);
        assert_eq!(
            store.review_report(id, &wallet).await.unwrap()["assessment"],
            assessment
        );
    }
}

impl PostgresStore {
    pub async fn artifact_internal(&self, id: Uuid) -> Result<Artifact> {
        artifact(sqlx::query("SELECT * FROM verification_artifacts WHERE id=$1 AND status NOT IN ('deleting','deleted')").bind(id).fetch_optional(&self.pool).await?.ok_or(EvidenceError::NotFound)?)
    }
    /// Only call after matching immutable indexed submission evidence and solver authority.
    pub async fn confirm_artifact(&self, id: Uuid, owner: &str, scope: &Value) -> Result<Artifact> {
        // Publication permission can change; the canonical commitment cannot.
        artifact(sqlx::query("UPDATE verification_artifacts SET canonical_scope=$3,pending_scope=NULL,updated_at=now() WHERE id=$1 AND owner_key=$2 AND status='ready' AND (canonical_scope IS NULL OR canonical_scope-'visibility'=$3::jsonb-'visibility') RETURNING *").bind(id).bind(owner).bind(scope).fetch_optional(&self.pool).await?.ok_or(EvidenceError::Conflict)?)
    }
    pub async fn retain_closed_artifact(
        &self,
        id: Uuid,
        closed_at: DateTime<Utc>,
        winning: bool,
    ) -> Result<()> {
        sqlx::query("UPDATE verification_artifacts SET closed_at=COALESCE(closed_at,$2),retain_until=CASE WHEN closed_at IS NULL THEN $2+INTERVAL '1 year' ELSE retain_until END,winning=winning OR $3,updated_at=now() WHERE id=$1 AND canonical_scope IS NOT NULL AND status='ready'").bind(id).bind(closed_at).bind(winning).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn archive_candidates(&self) -> Result<Vec<Artifact>> {
        sqlx::query("UPDATE verification_artifacts SET updated_at=now() WHERE id IN (SELECT id FROM verification_artifacts WHERE canonical_scope IS NOT NULL AND status='ready' AND closed_at IS NULL ORDER BY updated_at LIMIT 100 FOR UPDATE SKIP LOCKED) RETURNING *").fetch_all(&self.pool).await?.into_iter().map(artifact).collect()
    }
    pub async fn pending_artifacts(&self) -> Result<Vec<Artifact>> {
        sqlx::query("UPDATE verification_artifacts SET updated_at=now() WHERE id IN (SELECT id FROM verification_artifacts WHERE pending_scope IS NOT NULL AND canonical_scope IS NULL AND status='ready' ORDER BY updated_at LIMIT 100 FOR UPDATE SKIP LOCKED) RETURNING *").fetch_all(&self.pool).await?.into_iter().map(artifact).collect()
    }
    /// Only after canonical events prove this exact round can no longer accept
    /// the prepared commitment. A timeout or missing RPC response is not proof.
    pub async fn release_unsubmitted_artifact(&self, id: Uuid, scope: &Value) -> Result<()> {
        sqlx::query("UPDATE verification_artifacts SET pending_scope=NULL,retain_until=GREATEST(retain_until,now()+INTERVAL '7 days'),updated_at=now() WHERE id=$1 AND pending_scope=$2 AND canonical_scope IS NULL AND status='ready'").bind(id).bind(scope).execute(&self.pool).await?;
        Ok(())
    }
    /// Never deletes live confirmed work, paid winners, disputes, pending submissions,
    /// or artifacts with queued/running checks. Claims a bounded batch for retryable cleanup.
    pub async fn expired_artifacts(&self) -> Result<Vec<Artifact>> {
        sqlx::query("UPDATE verification_artifacts SET status='deleting' WHERE id IN (SELECT a.id FROM verification_artifacts a WHERE a.status IN ('uploading','ready','deleting') AND NOT a.winning AND NOT a.dispute_hold AND a.pending_scope IS NULL AND (a.canonical_scope IS NULL OR a.closed_at IS NOT NULL) AND a.retain_until<now() AND NOT EXISTS(SELECT 1 FROM verification_runs r WHERE r.artifact_id=a.id AND r.status IN ('queued','running')) ORDER BY a.retain_until LIMIT 100 FOR UPDATE SKIP LOCKED) RETURNING *").fetch_all(&self.pool).await?.into_iter().map(artifact).collect()
    }
    pub async fn artifact_deleted(&self, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE verification_artifacts SET status='deleted',updated_at=now() WHERE id=$1 AND status='deleting' AND NOT winning AND NOT dispute_hold").bind(id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn save_review_report(
        &self,
        owner: &str,
        network: &str,
        request: &Value,
        assessment: &Value,
        previous: Option<Uuid>,
    ) -> Result<Value> {
        let contract = request["bounty_contract"]
            .as_str()
            .ok_or(EvidenceError::Invalid)?
            .to_lowercase();
        let reviewer = request["verifier"]
            .as_str()
            .ok_or(EvidenceError::Invalid)?
            .to_lowercase();
        let round = request["round"]
            .as_u64()
            .and_then(|n| i64::try_from(n).ok())
            .ok_or(EvidenceError::Invalid)?;
        let hash = request["response_hash"]
            .as_str()
            .ok_or(EvidenceError::Invalid)?;
        let mut tx = self.pool.begin().await?;
        if let Some(previous) = previous {
            let prior = self.review_report(previous, owner).await?;
            if prior["network"] != network
                || prior["bounty_contract"] != contract
                || prior["round"] != round
            {
                return Err(EvidenceError::NotFound);
            }
        }
        sqlx::query("INSERT INTO verification_review_reports(id,owner_key,network,bounty_contract,round,reviewer,assessment_hash,assessment,request,previous_report_id,request_hash) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT DO NOTHING")
            .bind(Uuid::new_v4()).bind(owner).bind(network).bind(&contract).bind(round).bind(&reviewer).bind(hash).bind(assessment).bind(request).bind(previous).bind(value_digest(request).map_err(|_|EvidenceError::Invalid)?).execute(&mut *tx).await?;
        let row=sqlx::query("SELECT id,owner_key,assessment,request FROM verification_review_reports WHERE network=$1 AND bounty_contract=$2 AND round=$3 AND reviewer=$4 AND assessment_hash=$5 AND request_hash=$6").bind(network).bind(contract).bind(round).bind(reviewer).bind(hash).bind(value_digest(request).map_err(|_|EvidenceError::Invalid)?).fetch_one(&mut *tx).await?;
        let id: Uuid = row.try_get("id")?;
        if row.try_get::<String, _>("owner_key")? != owner
            || row.try_get::<Value, _>("assessment")? != *assessment
            || row.try_get::<Value, _>("request")? != *request
        {
            return Err(EvidenceError::Conflict);
        }
        sqlx::query("INSERT INTO verification_report_events(id,report_id,phase,data) VALUES($1,$2,'draft',$3) ON CONFLICT DO NOTHING").bind(Uuid::new_v4()).bind(id).bind(assessment).execute(&mut *tx).await?;
        tx.commit().await?;
        self.review_report(id, owner).await
    }
    pub async fn review_report(&self, id: Uuid, owner: &str) -> Result<Value> {
        sqlx::query_scalar("SELECT to_jsonb(r)-'owner_key' FROM verification_review_reports r WHERE r.id=$1 AND (r.owner_key=$2 OR ('wallet:'||CASE WHEN r.network='base-mainnet' THEN '8453' WHEN r.network='base-sepolia' THEN '84532' ELSE 'unsupported' END||':'||r.reviewer)=$2 OR EXISTS(SELECT 1 FROM site_auth_wallets w WHERE 'account:'||w.account_key=$2 AND w.wallet_address=r.reviewer AND w.chain_id=CASE WHEN r.network='base-mainnet' THEN 8453 WHEN r.network='base-sepolia' THEN 84532 ELSE 0 END AND w.proof_method='eip191_personal_sign'))").bind(id).bind(owner).fetch_optional(&self.pool).await?.ok_or(EvidenceError::NotFound)
    }
    pub async fn review_report_phase(
        &self,
        id: Uuid,
        owner: &str,
        phase: &str,
        data: &Value,
    ) -> Result<Value> {
        if !matches!(phase, "signed" | "submitted" | "confirmed") {
            return Err(EvidenceError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let old:String=sqlx::query_scalar("SELECT r.status FROM verification_review_reports r WHERE r.id=$1 AND (r.owner_key=$2 OR ('wallet:'||CASE WHEN r.network='base-mainnet' THEN '8453' WHEN r.network='base-sepolia' THEN '84532' ELSE 'unsupported' END||':'||r.reviewer)=$2 OR EXISTS(SELECT 1 FROM site_auth_wallets w WHERE 'account:'||w.account_key=$2 AND w.wallet_address=r.reviewer AND w.chain_id=CASE WHEN r.network='base-mainnet' THEN 8453 WHEN r.network='base-sepolia' THEN 84532 ELSE 0 END AND w.proof_method='eip191_personal_sign')) FOR UPDATE OF r").bind(id).bind(owner).fetch_optional(&mut *tx).await?.ok_or(EvidenceError::NotFound)?;
        if let Some(existing) = sqlx::query_scalar::<_, Value>(
            "SELECT data FROM verification_report_events WHERE report_id=$1 AND phase=$2",
        )
        .bind(id)
        .bind(phase)
        .fetch_optional(&mut *tx)
        .await?
        {
            if existing != *data {
                return Err(EvidenceError::Conflict);
            }
            tx.commit().await?;
            return self.review_report(id, owner).await;
        }
        if !matches!(
            (old.as_str(), phase),
            ("draft", "signed")
                | ("signed", "submitted")
                | ("signed", "confirmed")
                | ("submitted", "confirmed")
        ) {
            return Err(EvidenceError::Conflict);
        }
        sqlx::query(
            "INSERT INTO verification_report_events(id,report_id,phase,data) VALUES($1,$2,$3,$4)",
        )
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(phase)
        .bind(data)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE verification_review_reports r SET status=$3,signed_attestation=CASE WHEN $3='signed' THEN $4 ELSE signed_attestation END,transaction_hash=CASE WHEN $3 IN ('submitted','confirmed') THEN $4->>'transaction_hash' ELSE transaction_hash END,canonical_event_id=CASE WHEN $3='confirmed' THEN $4->>'canonical_event_id' ELSE canonical_event_id END WHERE r.id=$1 AND (r.owner_key=$2 OR ('wallet:'||CASE WHEN r.network='base-mainnet' THEN '8453' WHEN r.network='base-sepolia' THEN '84532' ELSE 'unsupported' END||':'||r.reviewer)=$2 OR EXISTS(SELECT 1 FROM site_auth_wallets w WHERE 'account:'||w.account_key=$2 AND w.wallet_address=r.reviewer AND w.chain_id=CASE WHEN r.network='base-mainnet' THEN 8453 WHEN r.network='base-sepolia' THEN 84532 ELSE 0 END AND w.proof_method='eip191_personal_sign'))").bind(id).bind(owner).bind(phase).bind(data).execute(&mut *tx).await?;
        tx.commit().await?;
        self.review_report(id, owner).await
    }
}

impl PostgresStore {
    pub async fn prepare_artifact_scope(&self, id: Uuid, owner: &str, scope: &Value) -> Result<()> {
        let changed=sqlx::query("UPDATE verification_artifacts SET pending_scope=$3,updated_at=now() WHERE id=$1 AND owner_key=$2 AND status='ready' AND canonical_scope IS NULL AND (pending_scope IS NULL OR pending_scope=$3)").bind(id).bind(owner).bind(scope).execute(&self.pool).await?;
        if changed.rows_affected() != 1 {
            return Err(EvidenceError::Conflict);
        }
        Ok(())
    }
    pub async fn published_artifacts(
        &self,
        network: &str,
        contract: &str,
    ) -> Result<Vec<Artifact>> {
        sqlx::query("SELECT * FROM verification_artifacts WHERE canonical_scope->>'network'=$1 AND canonical_scope->>'bounty_contract'=$2 AND canonical_scope->>'visibility'='public' AND status='ready'").bind(network).bind(contract.to_lowercase()).fetch_all(&self.pool).await?.into_iter().map(artifact).collect()
    }
    pub async fn published_reviews(&self, network: &str, contract: &str) -> Result<Vec<Value>> {
        Ok(sqlx::query_scalar("SELECT jsonb_build_object('id',id,'round',round,'reviewer',reviewer,'assessment_hash',assessment_hash,'assessment',assessment,'status',status,'transaction_hash',transaction_hash,'canonical_event_id',canonical_event_id) FROM verification_review_reports WHERE network=$1 AND bounty_contract=$2 AND status='confirmed' ORDER BY round,created_at").bind(network).bind(contract.to_lowercase()).fetch_all(&self.pool).await?)
    }
}
