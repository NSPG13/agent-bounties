//! Private acquisition data and durable measurement delivery. Never payment authority.
use super::{DbResult, PostgresStore};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

pub struct NewClick<'a> {
    pub acquisition_id: Uuid,
    pub hash: &'a str,
    pub kind: &'a str,
    pub ciphertext: &'a str,
    pub campaign: &'a str,
    pub now: DateTime<Utc>,
}

#[derive(Clone)]
pub struct Delivery {
    pub id: Uuid,
    pub lease_id: Uuid,
    pub kind: String,
    pub ciphertext: String,
    pub outcome: String,
    pub event_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct CampaignRow {
    pub campaign: String,
    pub first_touch_funded_posters: i64,
    pub first_touch_funded_bounties: i64,
    pub first_touch_paid_bounties: i64,
    pub google_assisted_funded_bounties: i64,
    pub cost_mxn: Option<f64>,
    pub clicks: Option<i64>,
    pub cost_per_funded_poster_mxn: Option<f64>,
    pub cost_per_paid_bounty_mxn: Option<f64>,
    pub cost_days_available: i64,
    pub funnel: Value,
    pub funded_in_window_lifecycle_as_of_now: Value,
    pub deliveries: Value,
}

impl PostgresStore {
    pub async fn google_ads_canary_matches(&self, acquisition: Uuid, kind: &str) -> DbResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM distribution_acquisitions WHERE id=$1 AND canary_kind=$2 AND measurement_eligible=FALSE)").bind(acquisition).bind(kind).fetch_one(&self.pool).await?)
    }

    pub async fn google_ads_pin_destination(
        &self,
        item: &Delivery,
        customer: &str,
        action: &str,
    ) -> DbResult<bool> {
        Ok(sqlx::query("UPDATE google_ads_outbox SET destination_customer=$3,destination_action=$4 WHERE id=$1 AND lease_id=$2 AND status='leased' AND (destination_customer IS NULL OR (destination_customer=$3 AND destination_action=$4))")
            .bind(item.id).bind(item.lease_id).bind(customer).bind(action).execute(&self.pool).await?.rows_affected()==1)
    }

    pub async fn google_ads_acquisition(&self, hash: &str) -> DbResult<Option<Uuid>> {
        Ok(
            sqlx::query_scalar("SELECT id FROM distribution_acquisitions WHERE token_hash=$1")
                .bind(hash)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn google_ads_record_click(
        &self,
        click: NewClick<'_>,
    ) -> DbResult<Option<DateTime<Utc>>> {
        // A repeated or copied click cannot move between acquisitions or renew consent/expiry.
        let mut tx = self.pool.begin().await?;
        // Serialize the per-acquisition storage quota, including concurrent requests.
        sqlx::query("SELECT id FROM distribution_acquisitions WHERE id=$1 FOR UPDATE")
            .bind(click.acquisition_id)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO google_ads_clicks (id,acquisition_id,click_hash,identifier_kind,ciphertext,campaign,consent_version,observed_at,expires_at) SELECT $1,$2,$3,$4,$5,$6,'google-ads-outcomes-v1',$7,$7+interval '90 days' WHERE (SELECT count(*) FROM google_ads_clicks WHERE acquisition_id=$2 AND expires_at>now())<100 ON CONFLICT(click_hash) DO NOTHING")
            .bind(Uuid::new_v4()).bind(click.acquisition_id).bind(click.hash).bind(click.kind)
            .bind(click.ciphertext).bind(click.campaign).bind(click.now).execute(&mut *tx).await?;
        let expires=sqlx::query_scalar("SELECT expires_at FROM google_ads_clicks WHERE click_hash=$1 AND acquisition_id=$2 AND revoked_at IS NULL AND expires_at>now() AND ciphertext IS NOT NULL")
            .bind(click.hash).bind(click.acquisition_id).fetch_optional(&mut *tx).await?;
        if expires.is_some() {
            sqlx::query("INSERT INTO distribution_acquisition_assists(acquisition_id,rail,first_observed_at,last_observed_at) SELECT $1,'google-ads',$2,$2 FROM distribution_acquisitions a WHERE a.id=$1 AND a.first_touch_rail<>'google-ads' ON CONFLICT(acquisition_id,rail) DO UPDATE SET last_observed_at=GREATEST(distribution_acquisition_assists.last_observed_at,EXCLUDED.last_observed_at)").bind(click.acquisition_id).bind(click.now).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(expires)
    }

    pub async fn google_ads_consent_active(&self, acquisition: Uuid) -> DbResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM google_ads_clicks WHERE acquisition_id=$1 AND revoked_at IS NULL AND expires_at>now() AND ciphertext IS NOT NULL)").bind(acquisition).fetch_one(&self.pool).await?)
    }

    pub async fn google_ads_funnel(
        &self,
        acquisition: Uuid,
        operation: Uuid,
        stage: &str,
    ) -> DbResult<()> {
        sqlx::query("INSERT INTO google_ads_funnel(acquisition_id,operation_id,stage) SELECT $1,$2,$3 WHERE EXISTS(SELECT 1 FROM google_ads_clicks WHERE acquisition_id=$1 AND revoked_at IS NULL AND expires_at>now()) ON CONFLICT DO NOTHING")
            .bind(acquisition).bind(operation).bind(stage).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn google_ads_revoke(&self, acquisition: Uuid) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE google_ads_clicks SET ciphertext=NULL,click_hash=NULL,revoked_at=COALESCE(revoked_at,now()) WHERE acquisition_id=$1")
            .bind(acquisition).execute(&mut *tx).await?;
        sqlx::query("UPDATE google_ads_outbox SET status='suppressed',error_code='consent_withdrawn',lease_id=NULL,updated_at=now() WHERE click_id IN (SELECT id FROM google_ads_clicks WHERE acquisition_id=$1) AND status IN ('pending','leased')")
            .bind(acquisition).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn google_ads_reconcile(&self) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE google_ads_clicks SET ciphertext=NULL,click_hash=NULL WHERE expires_at<=now() AND (ciphertext IS NOT NULL OR click_hash IS NOT NULL)").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO google_ads_outbox(id,click_id,network,bounty_contract,outcome,event_at) SELECT gen_random_uuid(),e.click_id,e.network,e.bounty_contract,e.outcome,e.event_at FROM google_ads_eligible_outcomes e JOIN google_ads_clicks c ON c.id=e.click_id WHERE c.ciphertext IS NOT NULL AND c.expires_at>now() ON CONFLICT(network,bounty_contract,outcome) DO NOTHING").execute(&mut *tx).await?;
        sqlx::query("UPDATE google_ads_outbox q SET status='suppressed',error_code='no_longer_eligible',lease_id=NULL,updated_at=now() WHERE q.status IN ('pending','leased') AND NOT EXISTS(SELECT 1 FROM google_ads_eligible_outcomes e JOIN google_ads_clicks c ON c.id=e.click_id WHERE e.network=q.network AND e.bounty_contract=q.bounty_contract AND e.outcome=q.outcome AND e.click_id=q.click_id AND c.ciphertext IS NOT NULL AND c.expires_at>now())").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn google_ads_lease(&self) -> DbResult<Option<Delivery>> {
        let lease = Uuid::new_v4();
        let row = sqlx::query("WITH selected AS (SELECT q.id FROM google_ads_outbox q JOIN google_ads_clicks c ON c.id=q.click_id WHERE q.status IN ('pending','leased') AND q.next_attempt_at<=now() AND c.ciphertext IS NOT NULL AND c.revoked_at IS NULL AND c.expires_at>now() ORDER BY q.event_at FOR UPDATE OF q SKIP LOCKED LIMIT 1) UPDATE google_ads_outbox q SET status='leased',lease_id=$1,attempts=attempts+1,next_attempt_at=now()+interval '5 minutes',updated_at=now() FROM selected,google_ads_clicks c WHERE q.id=selected.id AND c.id=q.click_id RETURNING q.id,q.outcome,q.event_at,c.identifier_kind,c.ciphertext")
            .bind(lease).fetch_optional(&self.pool).await?;
        row.map(|r| {
            Ok(Delivery {
                id: r.try_get("id")?,
                lease_id: lease,
                kind: r.try_get("identifier_kind")?,
                ciphertext: r.try_get("ciphertext")?,
                outcome: r.try_get("outcome")?,
                event_at: r.try_get("event_at")?,
            })
        })
        .transpose()
    }

    pub async fn google_ads_delivery_valid(&self, item: &Delivery) -> DbResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM google_ads_outbox q JOIN google_ads_eligible_outcomes e ON e.network=q.network AND e.bounty_contract=q.bounty_contract AND e.outcome=q.outcome AND e.click_id=q.click_id JOIN google_ads_clicks c ON c.id=q.click_id WHERE q.id=$1 AND q.lease_id=$2 AND q.status='leased' AND c.revoked_at IS NULL AND c.expires_at>now() AND c.ciphertext IS NOT NULL)")
            .bind(item.id).bind(item.lease_id).fetch_one(&self.pool).await?)
    }

    pub async fn google_ads_complete(
        &self,
        item: &Delivery,
        status: &str,
        code: Option<&str>,
        request: Option<&str>,
    ) -> DbResult<()> {
        sqlx::query("UPDATE google_ads_outbox SET status=$3,error_code=$4,request_id=$5,lease_id=NULL,next_attempt_at=now()+make_interval(secs=>LEAST(3600,30*power(2,LEAST(attempts,7)))::double precision),updated_at=now() WHERE id=$1 AND lease_id=$2 AND status='leased'")
            .bind(item.id).bind(item.lease_id).bind(status).bind(code).bind(request).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn google_ads_cost(
        &self,
        campaign: &str,
        day: chrono::NaiveDate,
        micros: i64,
        clicks: i64,
    ) -> DbResult<()> {
        sqlx::query("INSERT INTO google_ads_campaign_costs(campaign,day,currency,cost_micros,clicks,source) VALUES($1,$2,'MXN',$3,$4,'google_ads_export') ON CONFLICT(campaign,day) DO UPDATE SET cost_micros=EXCLUDED.cost_micros,clicks=EXCLUDED.clicks,imported_at=now()")
            .bind(campaign).bind(day).bind(micros).bind(clicks).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn google_ads_diagnostics_due(&self) -> DbResult<Vec<(Uuid, String)>> {
        Ok(sqlx::query_as("SELECT id,request_id FROM google_ads_outbox WHERE status='accepted' AND request_id IS NOT NULL AND COALESCE(processing_status,'PROCESSING') NOT IN ('SUCCESS','PARTIAL_SUCCESS','FAILED') AND updated_at < now()-interval '30 minutes' AND (diagnostics_checked_at IS NULL OR diagnostics_checked_at < now()-interval '1 hour') ORDER BY updated_at LIMIT 20").fetch_all(&self.pool).await?)
    }

    pub async fn google_ads_diagnostics_save(
        &self,
        id: Uuid,
        status: &str,
        errors: i32,
        warnings: i32,
        reasons: Value,
    ) -> DbResult<()> {
        sqlx::query("UPDATE google_ads_outbox SET processing_status=$2,processing_errors=$3,processing_warnings=$4,processing_reasons=$5,diagnostics_checked_at=now() WHERE id=$1 AND status='accepted'")
            .bind(id).bind(status).bind(errors).bind(warnings).bind(reasons).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn google_ads_report(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> DbResult<Value> {
        let rows = sqlx::query(include_str!("google_ads_report.sql"))
            .bind(start)
            .bind(end)
            .fetch_all(&self.pool)
            .await?;
        let mut campaigns = Vec::new();
        let complete_days = start.time() == chrono::NaiveTime::from_hms_opt(6, 0, 0).unwrap()
            && end.time() == chrono::NaiveTime::from_hms_opt(6, 0, 0).unwrap();
        let expected_days = (end - start).num_days();
        for r in rows {
            let campaign: String = r.try_get("campaign")?;
            let posters: i64 = r.try_get("posters")?;
            let paid: i64 = r.try_get("paid")?;
            let cost: Option<f64> = r.try_get("cost")?;
            let cost_days: i64 = r.try_get("cost_days")?;
            let comparable_cost = cost.filter(|_| complete_days && cost_days == expected_days);
            let funnel: Value=sqlx::query_scalar("SELECT COALESCE(jsonb_object_agg(stage,n),'{}') FROM (SELECT f.stage,count(DISTINCT(f.acquisition_id,f.operation_id)) AS n FROM google_ads_funnel f JOIN distribution_acquisitions a ON a.id=f.acquisition_id WHERE a.measurement_eligible=TRUE AND a.first_touch_rail='google-ads' AND f.observed_at >= $2 AND f.observed_at < $3 AND (SELECT campaign FROM google_ads_clicks c WHERE c.acquisition_id=f.acquisition_id ORDER BY observed_at,id LIMIT 1)=$1 GROUP BY f.stage) t").bind(&campaign).bind(start).bind(end).fetch_one(&self.pool).await?;
            let deliveries:Value=sqlx::query_scalar("SELECT COALESCE(jsonb_agg(t),'[]') FROM (SELECT q.status,q.error_code,q.processing_status,q.processing_errors,q.processing_warnings,q.processing_reasons,count(*) AS outcomes FROM google_ads_outbox q JOIN google_ads_clicks c ON c.id=q.click_id WHERE c.campaign=$1 AND q.event_at >= $2 AND q.event_at < $3 GROUP BY q.status,q.error_code,q.processing_status,q.processing_errors,q.processing_warnings,q.processing_reasons) t").bind(&campaign).bind(start).bind(end).fetch_one(&self.pool).await?;
            let lifecycle: Value = sqlx::query_scalar(include_str!("google_ads_lifecycle.sql"))
                .bind(&campaign)
                .bind(start)
                .bind(end)
                .fetch_one(&self.pool)
                .await?;
            campaigns.push(CampaignRow {
                campaign,
                first_touch_funded_posters: posters,
                first_touch_funded_bounties: r.try_get("funded")?,
                first_touch_paid_bounties: paid,
                google_assisted_funded_bounties: r.try_get("assisted")?,
                cost_mxn: cost,
                clicks: r.try_get("clicks")?,
                cost_per_funded_poster_mxn: comparable_cost
                    .filter(|_| posters > 0)
                    .map(|c| c / posters as f64),
                cost_per_paid_bounty_mxn: comparable_cost
                    .filter(|_| paid > 0)
                    .map(|c| c / paid as f64),
                cost_days_available: cost_days,
                funnel,
                funded_in_window_lifecycle_as_of_now: lifecycle,
                deliveries,
            });
        }
        let invalidated:i64=sqlx::query_scalar("SELECT count(*) FROM google_ads_outbox q WHERE status='accepted' AND NOT EXISTS(SELECT 1 FROM google_ads_eligible_outcomes e WHERE e.network=q.network AND e.bounty_contract=q.bounty_contract AND e.outcome=q.outcome AND e.click_id=q.click_id)").fetch_one(&self.pool).await?;
        let tracking:Value=sqlx::query_scalar(r#"SELECT jsonb_build_object(
          'canonical_external_funded',count(*),
          'with_proven_google_join',count(*) FILTER(WHERE EXISTS(SELECT 1 FROM google_ads_attributed_outcomes e WHERE e.network=o.network AND e.bounty_contract=o.bounty_contract)),
          'no_proven_google_join',count(*) FILTER(WHERE NOT EXISTS(SELECT 1 FROM google_ads_attributed_outcomes e WHERE e.network=o.network AND e.bounty_contract=o.bounty_contract))
        ) FROM google_ads_canonical_outcomes o WHERE o.funded_at >= $1 AND o.funded_at < $2"#).bind(start).bind(end).fetch_one(&self.pool).await?;
        let coverage:Value=sqlx::query_scalar(r#"WITH journeys AS (
          SELECT f.*,EXISTS(SELECT 1 FROM distribution_acquisition_handoffs h
            WHERE h.acquisition_id=f.acquisition_id AND h.request_fingerprint=encode(sha256(convert_to('google-ads-posting-operation:'||f.operation_id::text,'UTF8')),'hex')
            AND h.terms_hash IS NOT NULL AND h.creator_wallet IS NOT NULL) AS bound
          FROM google_ads_funnel f JOIN distribution_acquisitions a ON a.id=f.acquisition_id
          WHERE f.stage='wallet_review' AND a.measurement_eligible=TRUE AND f.observed_at >= $1 AND f.observed_at < $2
          AND EXISTS(SELECT 1 FROM google_ads_clicks c WHERE c.acquisition_id=f.acquisition_id AND c.revoked_at IS NULL)
          AND NOT EXISTS(SELECT 1 FROM distribution_acquisition_handoffs h JOIN distribution_wallet_exclusions x ON x.wallet_address=h.creator_wallet AND x.active=TRUE WHERE h.acquisition_id=f.acquisition_id)
        ) SELECT jsonb_build_object('consented_wallet_review_journeys',count(*),'bound_journeys',count(*) FILTER(WHERE bound),
          'basis_points',CASE WHEN count(*)>0 THEN 10000*(count(*) FILTER(WHERE bound))/count(*) ELSE NULL END) FROM journeys"#).bind(start).bind(end).fetch_one(&self.pool).await?;
        Ok(
            json!({"schema_version":"agent-bounties/google-ads-report-v1","protocol_scope":"agent-bounties/autonomous-v1","window_start":start,"window_end_exclusive":end,"campaigns":campaigns,"accepted_now_ineligible":invalidated,"google_credited_results":null,"known_journey_join_coverage":coverage,"tracking":tracking,"coverage_note":"Coverage is limited to consented, captured, eligible wallet-review journeys. Capture failures and declined consent cannot be reconstructed; no proven Google join is not proof of missing Google traffic. Full controlled-journey testing is a separate launch gate.","evidence_boundary":"Canonical outcomes by event date, including outcomes outside Google's upload window. Original Google campaign and latest-click assists are separate from Google's attribution. Accepted/processed uploads are not credited conversions. Distinct wallets are not distinct people. Cost imports use verified account UTC-06:00 days; CAC is null unless every full day is imported. No inferred historical credit."}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use sha2::{Digest, Sha256};

    async fn fixture(
        store: &PostgresStore,
        canary: bool,
        age: i64,
    ) -> (Uuid, String, String, String, Uuid) {
        let nonce = Uuid::new_v4().simple().to_string();
        let hash = hex::encode(Sha256::digest(&nonce));
        let wallet = format!("0x{}", &hash[..40]);
        let bounty = format!(
            "0x{}",
            &hex::encode(Sha256::digest(format!("bounty:{nonce}")))[..40]
        );
        let terms = format!("0x{hash}");
        let now = Utc::now();
        let click_at = now - Duration::days(age) - Duration::minutes(10);
        let a = store
            .observe_distribution_acquisition(
                "google-ads",
                &hash,
                if canary { Some("dry-run-v1") } else { None },
                click_at,
            )
            .await
            .unwrap();
        store
            .google_ads_record_click(NewClick {
                acquisition_id: a.id,
                hash: &hash,
                kind: "gclid",
                ciphertext: "encrypted-fixture",
                campaign: "campaign-first",
                now: click_at,
            })
            .await
            .unwrap();
        let operation = Uuid::new_v4();
        let fingerprint = hex::encode(Sha256::digest(format!(
            "google-ads-posting-operation:{operation}"
        )));
        let h = store
            .reserve_distribution_handoff(a.id, &fingerprint, now - Duration::minutes(8))
            .await
            .unwrap();
        sqlx::query("UPDATE distribution_acquisition_handoffs SET terms_hash=$2,creator_wallet=$3,terms_bound_at=$4,wallet_reviewed_at=$4 WHERE id=$1").bind(h.id).bind(&terms).bind(&wallet).bind(now-Duration::minutes(7)).execute(&store.pool).await.unwrap();
        let events = [
            (
                "canonical_bounty_created",
                json!({"creator":wallet,"terms_hash":terms,"bounty_contract":bounty}),
            ),
            (
                "canonical_bounty_economics_configured",
                json!({"target_amount":"2100000"}),
            ),
            (
                "funding_added",
                json!({"contributor":wallet,"amount":"2100000"}),
            ),
            ("bounty_became_claimable", json!({})),
            (
                "bounty_settled",
                json!({"solver":"0x9999999999999999999999999999999999999999","round":"1","evidence_hash":format!("0x{}","e".repeat(64))}),
            ),
        ];
        for (index, (kind, data)) in events.into_iter().enumerate() {
            sqlx::query("INSERT INTO autonomous_bounty_events(id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data,occurred_at,block_time_verified) VALUES($1,$2,'base-mainnet',$3,$4,0,$5,$5,$6,$7,$8,TRUE)")
                .bind(Uuid::new_v4()).bind(format!("{nonce}:{index}")).bind(format!("0x{hash}")).bind(index as i64+1).bind(&bounty).bind(kind).bind(data).bind(now-Duration::minutes(6)+Duration::seconds(index as i64)).execute(&store.pool).await.unwrap();
        }
        (a.id, bounty, wallet, hash, operation)
    }

    #[tokio::test]
    #[ignore = "requires disposable AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn google_ads_postgres_journey_and_delivery_boundaries() {
        let store =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        store.migrate().await.unwrap();
        let (acq, bounty, wallet, hash, operation) = fixture(&store, false, 0).await;
        // Re-running every startup migration must accept the added rails.
        store.migrate().await.unwrap();
        let original: DateTime<Utc> =
            sqlx::query_scalar("SELECT expires_at FROM google_ads_clicks WHERE acquisition_id=$1")
                .bind(acq)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        let repeated = store
            .google_ads_record_click(NewClick {
                acquisition_id: acq,
                hash: &hash,
                kind: "gclid",
                ciphertext: "replacement-not-used",
                campaign: "must-not-overwrite",
                now: Utc::now(),
            })
            .await
            .unwrap();
        assert_eq!(repeated, Some(original));
        // Confirmed settlement without matching evidence is not a paid conversion.
        let counts: Vec<String> = sqlx::query_scalar(
            "SELECT outcome FROM google_ads_eligible_outcomes WHERE bounty_contract=$1",
        )
        .bind(&bounty)
        .fetch_all(&store.pool)
        .await
        .unwrap();
        assert_eq!(counts, vec!["external_bounty_funded"]);
        sqlx::query("UPDATE autonomous_bounty_events SET data=jsonb_set(data,'{amount}','\"1\"') WHERE bounty_id=$1 AND kind='funding_added'").bind(&bounty).execute(&store.pool).await.unwrap();
        let insufficient: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM google_ads_eligible_outcomes WHERE bounty_contract=$1",
        )
        .bind(&bounty)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(insufficient, 0);
        sqlx::query("UPDATE autonomous_bounty_events SET data=jsonb_set(data,'{amount}','\"2100000\"') WHERE bounty_id=$1 AND kind='funding_added'").bind(&bounty).execute(&store.pool).await.unwrap();
        sqlx::query("INSERT INTO autonomous_submission_evidence(network,bounty_contract,bounty_id,round,solver_wallet,artifact_reference,artifact_hash,evidence,evidence_hash) VALUES('base-mainnet',$1,$1,1,'0x9999999999999999999999999999999999999999','https://example.org/fixture',$2,'{}',$2)").bind(&bounty).bind(format!("0x{}","e".repeat(64))).execute(&store.pool).await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM google_ads_eligible_outcomes WHERE bounty_contract=$1",
        )
        .bind(&bounty)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 2);
        sqlx::query("UPDATE autonomous_bounty_events SET block_time_verified=FALSE WHERE bounty_id=$1 AND kind='bounty_settled'").bind(&bounty).execute(&store.pool).await.unwrap();
        let unconfirmed: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM google_ads_eligible_outcomes WHERE bounty_contract=$1",
        )
        .bind(&bounty)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(unconfirmed, 1);
        sqlx::query("UPDATE autonomous_bounty_events SET block_time_verified=TRUE WHERE bounty_id=$1 AND kind='bounty_settled'").bind(&bounty).execute(&store.pool).await.unwrap();
        store.google_ads_reconcile().await.unwrap();
        store.google_ads_reconcile().await.unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM google_ads_outbox WHERE bounty_contract=$1")
                .bind(&bounty)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(count, 2);
        let first = store.google_ads_lease().await.unwrap().unwrap();
        let second = store.google_ads_lease().await.unwrap().unwrap();
        assert_ne!(first.id, second.id);
        assert!(store.google_ads_lease().await.unwrap().is_none());
        assert!(store.google_ads_delivery_valid(&first).await.unwrap());
        assert!(store
            .google_ads_pin_destination(&first, "1234567890", "111")
            .await
            .unwrap());
        assert!(store
            .google_ads_pin_destination(&first, "1234567890", "111")
            .await
            .unwrap());
        assert!(!store
            .google_ads_pin_destination(&first, "1234567890", "222")
            .await
            .unwrap());
        store
            .google_ads_complete(&first, "pending", Some("google_transport"), None)
            .await
            .unwrap();
        assert!(store.google_ads_lease().await.unwrap().is_none()); // backoff, no hot loop
        sqlx::query(
            "UPDATE google_ads_outbox SET next_attempt_at=now()-interval '1 second' WHERE id=$1",
        )
        .bind(first.id)
        .execute(&store.pool)
        .await
        .unwrap();
        let retry = store.google_ads_lease().await.unwrap().unwrap();
        assert_eq!(retry.id, first.id);
        assert_ne!(retry.lease_id, first.lease_id);
        store
            .google_ads_complete(&first, "accepted", None, Some("stale-worker"))
            .await
            .unwrap();
        assert!(store.google_ads_delivery_valid(&retry).await.unwrap());
        store
            .google_ads_complete(&retry, "accepted", None, Some("request-fixture"))
            .await
            .unwrap();
        store
            .google_ads_diagnostics_save(
                retry.id,
                "FAILED",
                1,
                0,
                json!([{"reason":"PROCESSING_ERROR_REASON_CLICK_NOT_FOUND","count":1}]),
            )
            .await
            .unwrap();
        store
            .google_ads_funnel(acq, operation, "wallet_review")
            .await
            .unwrap();
        let report = store
            .google_ads_report(Utc::now() - Duration::days(1), Utc::now())
            .await
            .unwrap();
        assert_eq!(report["protocol_scope"], "agent-bounties/autonomous-v1");
        assert!(report["google_credited_results"].is_null());
        assert_eq!(report["known_journey_join_coverage"]["basis_points"], 10000);
        // A later campaign assists but never rewrites the original campaign.
        store
            .google_ads_record_click(NewClick {
                acquisition_id: acq,
                hash: &format!("later-{hash}"),
                kind: "gclid",
                ciphertext: "later-fixture",
                campaign: "campaign-later",
                now: Utc::now() - Duration::minutes(9),
            })
            .await
            .unwrap();
        let source:(String,String)=sqlx::query_as("SELECT first_touch_campaign,campaign FROM google_ads_attributed_outcomes WHERE bounty_contract=$1 LIMIT 1").bind(&bounty).fetch_one(&store.pool).await.unwrap();
        assert_eq!(source, ("campaign-first".into(), "campaign-later".into()));
        // Operator classification takes effect even after a worker leased a row.
        sqlx::query("INSERT INTO distribution_wallet_exclusions(wallet_address,exclusion_class) VALUES($1,'operator')").bind(&wallet).execute(&store.pool).await.unwrap();
        assert!(!store.google_ads_delivery_valid(&second).await.unwrap());
        store.google_ads_reconcile().await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM google_ads_canonical_outcomes WHERE bounty_contract=$1",
        )
        .bind(&bounty)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
        let (canary, canary_bounty, _, _, _) = fixture(&store, true, 0).await;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM google_ads_eligible_outcomes WHERE bounty_contract=$1",
        )
        .bind(&canary_bounty)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
        store.google_ads_revoke(canary).await.unwrap();
        assert!(!store.google_ads_consent_active(canary).await.unwrap());
        store.google_ads_revoke(acq).await.unwrap();
        let retained:i64=sqlx::query_scalar("SELECT count(*) FROM google_ads_clicks WHERE acquisition_id=$1 AND (ciphertext IS NOT NULL OR click_hash IS NOT NULL)").bind(acq).fetch_one(&store.pool).await.unwrap();
        assert_eq!(retained, 0);
        let (old, old_bounty, _, _, _) = fixture(&store, false, 95).await;
        store.google_ads_reconcile().await.unwrap();
        assert!(!store.google_ads_consent_active(old).await.unwrap());
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM google_ads_eligible_outcomes WHERE bounty_contract=$1",
        )
        .bind(&old_bounty)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM google_ads_attributed_outcomes WHERE bounty_contract=$1",
        )
        .bind(&old_bounty)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 1); // still reported internally
    }
}
