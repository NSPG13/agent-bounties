use super::*;
impl PostgresStore {
    pub async fn creator_open_public_evidence(
        &self,
        network: &str,
        bounty: &str,
        hashes: &[String],
    ) -> DbResult<
        std::collections::BTreeMap<
            String,
            chain_base::creator_open_service::CreatorOpenPublicEvidence,
        >,
    > {
        let rows=sqlx::query("SELECT unsigned_request->'action'->>'evidence_hash' AS hash,unsigned_request->'public_evidence' AS evidence FROM creator_open_requests WHERE network=$1 AND bounty_contract=$2 AND unsigned_request->'action'->>'kind'='submit' AND unsigned_request->'action'->>'evidence_hash'=ANY($3) AND unsigned_request->'public_evidence' IS NOT NULL LIMIT 65")
            .bind(network).bind(bounty.to_ascii_lowercase()).bind(hashes).fetch_all(&self.pool).await?;
        let mut evidence = std::collections::BTreeMap::new();
        for row in rows {
            let hash: String = row.try_get("hash")?;
            let document: chain_base::creator_open_service::CreatorOpenPublicEvidence =
                serde_json::from_value(row.try_get("evidence")?)?;
            if document
                .commitment()
                .ok()
                .is_some_and(|h| format!("{h:#x}") == hash)
            {
                evidence.insert(hash, document);
            }
        }
        Ok(evidence)
    }
    /// This request table contains no signatures; execution envelopes use the private outbox.
    pub async fn save_creator_open_request(
        &self,
        network: &str,
        hash: &str,
        bounty: &str,
        id: Uuid,
        request: &chain_base::creator_open_service::Request,
    ) -> DbResult<()> {
        let request = serde_json::to_value(request)?;
        let row=sqlx::query("INSERT INTO creator_open_requests(network,request_hash,bounty_contract,action_id,unsigned_request) VALUES($1,$2,$3,$4,$5) ON CONFLICT(network,request_hash) DO UPDATE SET request_hash=excluded.request_hash RETURNING bounty_contract,action_id,unsigned_request")
            .bind(network).bind(hash).bind(bounty.to_ascii_lowercase()).bind(id).bind(&request).fetch_one(&self.pool).await?;
        if row.try_get::<String, _>("bounty_contract")? != bounty.to_ascii_lowercase()
            || row.try_get::<Uuid, _>("action_id")? != id
            || row.try_get::<serde_json::Value, _>("unsigned_request")? != request
        {
            return Err(DbError::X402RelayQuotaExceeded(
                "creator-open request identity conflict".into(),
            ));
        }
        Ok(())
    }
    pub async fn creator_open_request(
        &self,
        network: &str,
        hash: &str,
    ) -> DbResult<Option<chain_base::creator_open_service::Request>> {
        sqlx::query("SELECT unsigned_request FROM creator_open_requests WHERE network=$1 AND request_hash=$2").bind(network).bind(hash).fetch_optional(&self.pool).await?.map(|row|Ok(serde_json::from_value(row.try_get("unsigned_request")?)?)).transpose()
    }
}

#[derive(Debug, Clone)]
pub struct CreatorOpenIndex {
    pub network: String,
    pub bounty_contract: String,
    pub request: chain_base::creator_open_service::Request,
    pub transaction_hash: String,
    pub version: i64,
    pub checkpoint: Option<chain_base::BaseBlockIdentity>,
    pub logs: Vec<chain_base::EvmLog>,
    pub projection: Option<chain_base::creator_open::Projection>,
    pub available: bool,
    pub checked_at: DateTime<Utc>,
}
fn indexed(row: PgRow) -> DbResult<CreatorOpenIndex> {
    Ok(CreatorOpenIndex {
        network: row.try_get("network")?,
        bounty_contract: row.try_get("bounty_contract")?,
        request: serde_json::from_value(row.try_get("unsigned_request")?)?,
        transaction_hash: row.try_get("transaction_hash")?,
        version: row.try_get("version")?,
        checkpoint: row
            .try_get::<Option<serde_json::Value>, _>("checkpoint")?
            .map(serde_json::from_value)
            .transpose()?,
        logs: serde_json::from_value(row.try_get("logs")?)?,
        projection: row
            .try_get::<Option<serde_json::Value>, _>("projection")?
            .map(serde_json::from_value)
            .transpose()?,
        available: row.try_get("available")?,
        checked_at: row.try_get("checked_at")?,
    })
}
impl PostgresStore {
    /// Public chain evidence is imported without allocating an action or spending
    /// a customer/relayer quota. Only the receipt-verifying API calls this writer.
    pub async fn save_creator_open_external_creation(
        &self,
        network: &str,
        bounty: &str,
        transaction_hash: &str,
        request: &chain_base::creator_open_service::Request,
    ) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        let bounty = bounty.to_ascii_lowercase();
        let hash = transaction_hash.to_ascii_lowercase();
        let request = serde_json::to_value(request)?;
        let row = sqlx::query("INSERT INTO creator_open_external_creations(id,network,bounty_contract,transaction_hash,unsigned_request) VALUES($1,$2,$3,$4,$5) ON CONFLICT(network,bounty_contract) DO UPDATE SET bounty_contract=excluded.bounty_contract RETURNING id,transaction_hash,unsigned_request")
            .bind(Uuid::new_v4()).bind(network).bind(&bounty).bind(&hash).bind(&request).fetch_one(&mut *tx).await?;
        if row.try_get::<String, _>("transaction_hash")? != hash
            || row.try_get::<serde_json::Value, _>("unsigned_request")? != request
        {
            return Err(DbError::X402RelayQuotaExceeded(
                "external creator-open identity conflict".into(),
            ));
        }
        sqlx::query("INSERT INTO creator_open_indexes(network,bounty_contract,external_creation_id) VALUES($1,$2,$3) ON CONFLICT(network,bounty_contract) DO NOTHING")
            .bind(network).bind(&bounty).bind(row.try_get::<Uuid,_>("id")?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn seed_creator_open_indexes(&self, network: &str) -> DbResult<()> {
        sqlx::query("INSERT INTO creator_open_indexes(network,bounty_contract,request_hash) SELECT r.network,r.bounty_contract,r.request_hash FROM creator_open_requests r JOIN sponsored_actions a ON a.id=r.action_id WHERE r.network=$1 AND r.unsigned_request->'action'->>'kind'='create' AND a.transaction_hash IS NOT NULL ON CONFLICT(network,bounty_contract) DO NOTHING").bind(network).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn creator_open_index_batch(
        &self,
        network: &str,
        bounty: Option<&str>,
    ) -> DbResult<Vec<CreatorOpenIndex>> {
        sqlx::query("SELECT i.*,COALESCE(r.unsigned_request,e.unsigned_request) AS unsigned_request,COALESCE(a.transaction_hash,e.transaction_hash) AS transaction_hash FROM creator_open_indexes i LEFT JOIN creator_open_requests r ON r.network=i.network AND r.request_hash=i.request_hash LEFT JOIN sponsored_actions a ON a.id=r.action_id LEFT JOIN creator_open_external_creations e ON e.id=i.external_creation_id AND e.network=i.network AND e.bounty_contract=i.bounty_contract WHERE i.network=$1 AND ($2::text IS NULL OR i.bounty_contract=$2) ORDER BY i.checked_at ASC LIMIT 50").bind(network).bind(bounty.map(str::to_ascii_lowercase)).fetch_all(&self.pool).await?.into_iter().map(indexed).collect()
    }
    /// Fence all cached evidence, including payment, immediately on a reorg/error.
    /// A prior concurrent poll cannot restore an older checkpoint after this version advances.
    pub async fn invalidate_creator_open_index(
        &self,
        index: &CreatorOpenIndex,
        reset: bool,
    ) -> DbResult<bool> {
        let r=sqlx::query("UPDATE creator_open_indexes SET available=false,checkpoint=CASE WHEN $4 THEN NULL ELSE checkpoint END,logs=CASE WHEN $4 THEN '[]'::jsonb ELSE logs END,projection=CASE WHEN $4 THEN NULL ELSE projection END,version=version+1,checked_at=now() WHERE network=$1 AND bounty_contract=$2 AND version=$3").bind(&index.network).bind(&index.bounty_contract).bind(index.version).bind(reset).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }
    pub async fn save_creator_open_index(
        &self,
        index: &CreatorOpenIndex,
        block: &chain_base::BaseBlockIdentity,
        logs: &[chain_base::EvmLog],
        projection: &chain_base::creator_open::Projection,
        available: bool,
    ) -> DbResult<bool> {
        let mut tx = self.pool.begin().await?;
        let r=sqlx::query("UPDATE creator_open_indexes SET checkpoint=$4,logs=$5,projection=$6,available=$7,version=version+1,checked_at=now() WHERE network=$1 AND bounty_contract=$2 AND version=$3").bind(&index.network).bind(&index.bounty_contract).bind(index.version).bind(serde_json::to_value(block)?).bind(serde_json::to_value(logs)?).bind(serde_json::to_value(projection)?).bind(available).execute(&mut *tx).await?;
        if r.rows_affected() == 1 {
            // The source view and jobs change in the same transaction as the
            // fenced projection. A concurrent old poll cannot restore notices.
            sqlx::query("INSERT INTO verifier_review_notifications AS j (id,network,bounty_contract,bounty_id,round,verifier_wallet,submission_log_key,submitted_at,review_deadline,last_seen_sync_id,notification_kind) SELECT gen_random_uuid(),network,bounty_contract,bounty_id,round,verifier_wallet,submission_log_key,submitted_at,review_deadline,gen_random_uuid(),'creator_open_submission' FROM creator_open_review_sources WHERE network=$1 AND bounty_contract=$2 ON CONFLICT(network,bounty_contract,round,verifier_wallet,notification_kind,submission_log_key) WHERE notification_kind='creator_open_submission' DO UPDATE SET source_active=(j.status IN ('pending','leased')),updated_at=now()")
                .bind(&index.network).bind(&index.bounty_contract).execute(&mut *tx).await?;
            sqlx::query("UPDATE verifier_review_notifications j SET source_active=FALSE,status='pending',lease_token=NULL,lease_expires_at=NULL,attempt_started=FALSE,updated_at=now() WHERE j.network=$1 AND j.bounty_contract=$2 AND j.notification_kind='creator_open_submission' AND j.status IN ('pending','leased') AND NOT EXISTS(SELECT 1 FROM creator_open_review_sources s WHERE s.network=j.network AND s.bounty_contract=j.bounty_contract AND s.round=j.round AND s.verifier_wallet=j.verifier_wallet AND s.submission_log_key=j.submission_log_key)")
                .bind(&index.network).bind(&index.bounty_contract).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(r.rows_affected() == 1)
    }
}
