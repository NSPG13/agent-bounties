use super::PostgresStore;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgRow, Row};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SitePostingDraft {
    pub operation_id: Uuid,
    pub draft: Value,
    pub recovery_state: Value,
    pub draft_hash: String,
    pub approved_draft_hash: Option<String>,
    pub revision: i64,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum PostingDraftError {
    #[error("posting draft storage unavailable")]
    Database(#[from] sqlx::Error),
    #[error("revision_conflict")]
    Conflict,
    #[error("draft_limit_reached")]
    Limit,
    #[error("invalid_draft_or_approval")]
    Invalid,
    #[error("draft_integer_exceeds_browser_safe_range_use_a_string")]
    NonCanonicalJson,
}

// RFC8785 uses ECMAScript number formatting and UTF-16 property ordering, matching
// the browser's sorted JSON including fractional visual annotations and Unicode.
// Approval binds only to this content. It never authorizes a wallet operation.
pub fn draft_hash(draft: &Value) -> Result<String, PostingDraftError> {
    fn canonical_subset(value: &Value, depth: usize) -> bool {
        if depth > 14 {
            return false;
        }
        match value {
            Value::Object(object) => object
                .values()
                .all(|value| canonical_subset(value, depth + 1)),
            Value::Array(items) => items.iter().all(|value| canonical_subset(value, depth + 1)),
            Value::Number(number) => number.as_f64().is_some_and(|value| {
                value.is_finite()
                    && (value.fract() != 0.0 || value.abs() <= 9_007_199_254_740_991.0)
            }),
            _ => true,
        }
    }
    if !canonical_subset(draft, 0) {
        return Err(PostingDraftError::NonCanonicalJson);
    }
    let encoded = serde_jcs::to_vec(draft).map_err(|_| PostingDraftError::Invalid)?;
    if !draft.is_object() || encoded.len() > 65_536 {
        return Err(PostingDraftError::Invalid);
    }
    Ok(Sha256::digest(encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn from_row(row: PgRow) -> Result<SitePostingDraft, sqlx::Error> {
    Ok(SitePostingDraft {
        operation_id: row.try_get("operation_id")?,
        draft: row.try_get("draft")?,
        recovery_state: row.try_get("recovery_state")?,
        draft_hash: row.try_get("draft_hash")?,
        approved_draft_hash: row.try_get("approved_draft_hash")?,
        revision: row.try_get("revision")?,
        updated_at: row.try_get("updated_at")?,
        expires_at: row.try_get("expires_at")?,
    })
}

impl PostgresStore {
    pub async fn get_site_posting_draft(
        &self,
        account_id: &str,
        operation_id: Uuid,
    ) -> Result<Option<SitePostingDraft>, PostingDraftError> {
        sqlx::query("SELECT operation_id, draft, recovery_state, draft_hash, approved_draft_hash, revision, updated_at, expires_at FROM site_posting_drafts WHERE account_id = $1 AND operation_id = $2 AND (expires_at > now() OR recovery_state ? 'bounty_id')")
            .bind(account_id).bind(operation_id).fetch_optional(&self.pool).await?
            .map(from_row).transpose().map_err(Into::into)
    }

    pub async fn save_site_posting_draft(
        &self,
        account_id: &str,
        operation_id: Uuid,
        draft: &Value,
        expected_revision: i64,
        approved_draft_hash: Option<&str>,
        recovery_state: &Value,
    ) -> Result<SitePostingDraft, PostingDraftError> {
        let hash = draft_hash(draft)?;
        if expected_revision < 0
            || approved_draft_hash.is_some_and(|approved| approved != hash)
            || !recovery_state.is_object()
            || serde_json::to_vec(recovery_state)
                .map_err(|_| PostingDraftError::Invalid)?
                .len()
                > 16_384
        {
            return Err(PostingDraftError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        // Serialize a single owner's edits and quota checks, including first insert.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 3671))")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM site_posting_drafts WHERE expires_at <= now() AND NOT (recovery_state ? 'bounty_id')")
            .execute(&mut *tx)
            .await?;
        let existing = sqlx::query("SELECT operation_id, draft, recovery_state, draft_hash, approved_draft_hash, revision, updated_at, expires_at FROM site_posting_drafts WHERE account_id = $1 AND operation_id = $2 FOR UPDATE")
            .bind(account_id).bind(operation_id).fetch_optional(&mut *tx).await?
            .map(from_row).transpose()?;
        if let Some(existing) = &existing {
            if existing.recovery_state.get("bounty_id").is_some()
                && (existing.draft_hash != hash
                    || existing.recovery_state.get("bounty_id") != recovery_state.get("bounty_id")
                    || existing.recovery_state.get("bounty_contract")
                        != recovery_state.get("bounty_contract")
                    || existing.recovery_state["transactions"]
                        .as_array()
                        .is_some_and(|transactions| {
                            transactions.iter().any(|id| {
                                !recovery_state["transactions"]
                                    .as_array()
                                    .is_some_and(|next| next.contains(id))
                            })
                        })
                    || existing.recovery_state["authorizationIssued"] == Value::Bool(true)
                        && recovery_state["authorizationIssued"] != Value::Bool(true))
            {
                return Err(PostingDraftError::Conflict);
            }
            // A lost response can be retried without incrementing revision or
            // creating a second posting operation. Never downgrade an approval.
            if existing.draft_hash == hash
                && existing.approved_draft_hash.as_deref() == approved_draft_hash
                && existing.recovery_state == *recovery_state
            {
                tx.commit().await?;
                return Ok(existing.clone());
            }
            if existing.revision != expected_revision {
                return Err(PostingDraftError::Conflict);
            }
        } else {
            if expected_revision != 0 {
                return Err(PostingDraftError::Conflict);
            }
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM site_posting_drafts WHERE account_id = $1 AND expires_at > now()",
            )
            .bind(account_id)
            .fetch_one(&mut *tx)
            .await?;
            if count >= 50 {
                return Err(PostingDraftError::Limit);
            }
        }
        let row = sqlx::query("INSERT INTO site_posting_drafts (account_id, operation_id, draft, draft_hash, approved_draft_hash, recovery_state, revision, expires_at) VALUES ($1, $2, $3, $4, $5, $6, 1, now() + interval '30 days') ON CONFLICT (account_id, operation_id) DO UPDATE SET draft = EXCLUDED.draft, draft_hash = EXCLUDED.draft_hash, approved_draft_hash = EXCLUDED.approved_draft_hash, recovery_state = EXCLUDED.recovery_state, revision = site_posting_drafts.revision + 1, updated_at = now(), expires_at = now() + interval '30 days' RETURNING operation_id, draft, recovery_state, draft_hash, approved_draft_hash, revision, updated_at, expires_at")
            .bind(account_id).bind(operation_id).bind(draft).bind(hash).bind(approved_draft_hash).bind(recovery_state)
            .fetch_one(&mut *tx).await?;
        let result = from_row(row)?;
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approval_hash_binds_nested_terms_and_is_key_order_independent() {
        let first: Value = serde_json::from_str(
            r#"{"title":"Video","image":{"sha256":"abc","url":"https://example.com"}}"#,
        )
        .unwrap();
        let reordered: Value = serde_json::from_str(
            r#"{"image":{"url":"https://example.com","sha256":"abc"},"title":"Video"}"#,
        )
        .unwrap();
        assert_eq!(draft_hash(&first).unwrap(), draft_hash(&reordered).unwrap());
        let mut changed = first.clone();
        changed["image"]["sha256"] = json!("changed");
        assert_ne!(draft_hash(&first).unwrap(), draft_hash(&changed).unwrap());
        assert!(draft_hash(&json!({"goal": "x".repeat(65_536)})).is_err());
        assert_eq!(
            draft_hash(
                &json!({"amount":"0.000001", "count":9_007_199_254_740_991i64, "title":"Vídeo 🌞"})
            )
            .unwrap(),
            "1d41456cb39a557eaf5f7ecf46d1f4e271a82739853539dde9852f0c97265fa5"
        );
        assert_eq!(
            draft_hash(&json!({"small":0.000001, "size":0.4, "zero":-0.0})).unwrap(),
            "b2b7a42de9ea755488182eab51d797d5fbe7d7acea401c9f48c082989cb5eb42"
        );
        assert_eq!(
            draft_hash(&json!({"\u{e000}":1,"\u{10000}":2})).unwrap(),
            "9d4cdc71dda603c42f9b21d88d0c2ffc31a76cd1bd461d7359406cf169845f1e"
        );
        for unsupported in [json!({"number":9_007_199_254_740_992u64})] {
            assert!(matches!(
                draft_hash(&unsupported),
                Err(PostingDraftError::NonCanonicalJson)
            ));
        }
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn account_drafts_roundtrip_replay_conflict_and_ownership() {
        let store =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        store.migrate().await.unwrap();
        let owner = Uuid::new_v4().to_string();
        let operation = Uuid::new_v4();
        let draft = json!({"goal":"Exact video", "review_mode":"creator", "delivery_deadline":"2026-09-10T21:00:00-06:00", "meta_child": null, "image":{"sha256":"approved-bytes"}});
        let recovery = json!({});
        let saved = store
            .save_site_posting_draft(&owner, operation, &draft, 0, None, &recovery)
            .await
            .unwrap();
        assert_eq!(
            saved,
            store
                .save_site_posting_draft(&owner, operation, &draft, 0, None, &recovery)
                .await
                .unwrap()
        );
        assert!(store
            .get_site_posting_draft("another-account", operation)
            .await
            .unwrap()
            .is_none());
        let approved = store
            .save_site_posting_draft(
                &owner,
                operation,
                &draft,
                saved.revision,
                Some(&saved.draft_hash),
                &recovery,
            )
            .await
            .unwrap();
        assert_eq!(
            approved.approved_draft_hash.as_deref(),
            Some(saved.draft_hash.as_str())
        );
        let changed = json!({"goal":"Other terms"});
        assert!(matches!(
            store
                .save_site_posting_draft(&owner, operation, &changed, 1, None, &recovery)
                .await,
            Err(PostingDraftError::Conflict)
        ));
        assert!(matches!(
            store
                .save_site_posting_draft(
                    &owner,
                    operation,
                    &changed,
                    2,
                    Some(&saved.draft_hash),
                    &recovery
                )
                .await,
            Err(PostingDraftError::Invalid)
        ));
        let updated = store
            .save_site_posting_draft(&owner, operation, &changed, 2, None, &recovery)
            .await
            .unwrap();
        assert!(updated.approved_draft_hash.is_none());
        assert_eq!(
            store
                .get_site_posting_draft(&owner, operation)
                .await
                .unwrap(),
            Some(updated.clone())
        );
        let pending = json!({"bounty_id":format!("0x{}", "11".repeat(32)), "phase":"sending"});
        let checkpoint = store
            .save_site_posting_draft(
                &owner,
                operation,
                &changed,
                updated.revision,
                None,
                &pending,
            )
            .await
            .unwrap();
        assert_eq!(checkpoint.draft_hash, updated.draft_hash);
        assert_eq!(checkpoint.recovery_state, pending);
        assert!(matches!(
            store
                .save_site_posting_draft(
                    &owner,
                    operation,
                    &draft,
                    checkpoint.revision,
                    None,
                    &pending
                )
                .await,
            Err(PostingDraftError::Conflict)
        ));
        assert_eq!(
            checkpoint,
            store
                .save_site_posting_draft(
                    &owner,
                    operation,
                    &changed,
                    updated.revision,
                    None,
                    &pending
                )
                .await
                .unwrap()
        );
        assert!(matches!(
            store
                .save_site_posting_draft(
                    &owner,
                    operation,
                    &changed,
                    updated.revision,
                    None,
                    &recovery
                )
                .await,
            Err(PostingDraftError::Conflict)
        ));
        sqlx::query("UPDATE site_posting_drafts SET expires_at = now() - interval '1 second' WHERE account_id = $1 AND operation_id = $2").bind(&owner).bind(operation).execute(&store.pool).await.unwrap();
        assert!(store
            .get_site_posting_draft(&owner, operation)
            .await
            .unwrap()
            .is_some());
        assert!(matches!(
            store
                .save_site_posting_draft(&owner, operation, &draft, 0, None, &recovery)
                .await,
            Err(PostingDraftError::Conflict)
        ));
        let unsent_operation = Uuid::new_v4();
        store
            .save_site_posting_draft(&owner, unsent_operation, &draft, 0, None, &recovery)
            .await
            .unwrap();
        sqlx::query("UPDATE site_posting_drafts SET expires_at = now() - interval '1 second' WHERE account_id = $1 AND operation_id = $2").bind(&owner).bind(unsent_operation).execute(&store.pool).await.unwrap();
        assert!(store
            .get_site_posting_draft(&owner, unsent_operation)
            .await
            .unwrap()
            .is_none());
        let restarted = store
            .save_site_posting_draft(&owner, unsent_operation, &draft, 0, None, &recovery)
            .await
            .unwrap();
        assert_eq!(restarted.revision, 1);
        assert!(restarted.approved_draft_hash.is_none());
    }
}
