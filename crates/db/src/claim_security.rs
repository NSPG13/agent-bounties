use crate::*;

impl PostgresStore {
    pub async fn active_claim_candidates(
        &self,
        network: &str,
        bounty: &str,
    ) -> DbResult<Vec<ClaimCandidate>> {
        sqlx::query("SELECT * FROM claim_candidates WHERE network=$1 AND bounty_contract=$2 AND status IN ('waitlisted','exclusive','authorization_ready','sponsoring','relaying') ORDER BY created_at,id LIMIT 100")
            .bind(network).bind(bounty).fetch_all(&self.pool).await?.into_iter().map(claim_candidate_from_row).collect()
    }

    pub async fn claim_candidate_principal(&self, id: Uuid) -> DbResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT principal_key FROM claim_candidates WHERE id=$1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    /// Fair bounded sweep selection across processes; per-bounty transitions use the queue lock.
    pub async fn claim_maintenance_batch(&self) -> DbResult<Vec<(String, String)>> {
        let rows = sqlx::query("WITH due AS (SELECT id FROM claim_candidates WHERE status IN ('waitlisted','exclusive','authorization_ready','sponsoring','relaying') AND (maintenance_checked_at IS NULL OR maintenance_checked_at<now()-interval '1 minute') ORDER BY maintenance_checked_at NULLS FIRST,created_at,id LIMIT 100 FOR UPDATE SKIP LOCKED) UPDATE claim_candidates c SET maintenance_checked_at=now() FROM due WHERE c.id=due.id RETURNING c.network,c.bounty_contract")
            .fetch_all(&self.pool).await?;
        let mut values = rows
            .into_iter()
            .map(|r| Ok((r.try_get("network")?, r.try_get("bounty_contract")?)))
            .collect::<DbResult<Vec<(String, String)>>>()?;
        values.sort();
        values.dedup();
        Ok(values)
    }

    pub async fn maintain_owned_claim_queue(
        &self,
        network: &str,
        bounty: &str,
        exclusive_seconds: u64,
        eligible: &[Uuid],
    ) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("claim-queue:{network}:{bounty}"))
            .execute(&mut *tx)
            .await?;
        // A process crash after broadcast must not make its reservation look abandoned.
        sqlx::query("UPDATE claim_candidates c SET claim_transaction_hash=r.tx_hash,status='relaying',updated_at=now() FROM x402_relay_attempts r WHERE c.network=$1 AND c.bounty_contract=$2 AND r.network=c.network AND r.bounty_contract=c.bounty_contract AND r.authorization_nonce=c.authorization_nonce AND r.tx_hash IS NOT NULL AND r.status IN ('broadcast','confirmed') AND c.status IN ('exclusive','authorization_ready','relaying') AND c.claim_transaction_hash IS NULL")
            .bind(network).bind(bounty).execute(&mut *tx).await?;
        // Owned relays durably enter `relaying` before a send-capable call. An unused
        // sponsorship therefore need not keep an expired owned reservation alive.
        // Historical, unattributed sponsorships retain their conservative protection.
        sqlx::query(r#"
            WITH expired AS (
              UPDATE claim_candidates c
              SET status='superseded',updated_at=now(),failure_code='reservation_expired'
              WHERE network=$1 AND bounty_contract=$2
                AND canonical_event_id IS NULL AND claim_transaction_hash IS NULL
                AND (relay_guard_until IS NULL OR relay_guard_until<=now())
                AND ((status='waitlisted' AND created_at<=now()-interval '24 hours')
                  OR (status IN ('exclusive','authorization_ready') AND exclusive_until<=now()))
                AND NOT EXISTS(SELECT 1 FROM bond_sponsorships b WHERE b.claim_candidate_id=c.id
                  AND (b.status IN ('broadcast','confirmed') OR b.transaction_hash IS NOT NULL
                    OR b.failure_code='broadcast_unknown'
                    OR (b.status='reserved' AND c.principal_key IS NULL)))
                AND NOT EXISTS(SELECT 1 FROM x402_relay_attempts r
                  WHERE r.network=c.network AND r.bounty_contract=c.bounty_contract
                    AND r.authorization_nonce=c.authorization_nonce
                    AND r.status IN ('prepared','relaying','broadcast','confirmed'))
              RETURNING c.id
            )
            UPDATE bond_sponsorships b SET status='failed',failure_code='reservation_expired',
              failure_message='The owned reservation expired before a relay attempt.',updated_at=now()
            FROM expired c WHERE b.claim_candidate_id=c.id AND b.status='reserved'
              AND b.transaction_hash IS NULL
        "#)
            .bind(network).bind(bounty).execute(&mut *tx).await?;
        sqlx::query("UPDATE claim_candidates c SET status='exclusive',exclusive_until=now()+make_interval(secs=>$3::double precision),updated_at=now() WHERE c.id=(SELECT id FROM claim_candidates w WHERE w.network=$1 AND w.bounty_contract=$2 AND w.status='waitlisted' AND w.principal_key IS NOT NULL AND w.created_at>now()-interval '24 hours' AND w.id=ANY($4) ORDER BY w.created_at,w.id LIMIT 1 FOR UPDATE) AND NOT EXISTS(SELECT 1 FROM claim_candidates a WHERE a.network=$1 AND a.bounty_contract=$2 AND a.status IN ('exclusive','authorization_ready','sponsoring','relaying'))")
            .bind(network).bind(bounty).bind(exclusive_seconds as f64).bind(eligible).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn withdraw_owned_claim_candidate(
        &self,
        id: Uuid,
        principal: &str,
    ) -> DbResult<ClaimCandidate> {
        let candidate = self
            .get_claim_candidate(id)
            .await?
            .ok_or_else(|| DbError::ClaimCandidateConflict("candidate not found".into()))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "claim-queue:{}:{}",
                candidate.network, candidate.bounty_contract
            ))
            .execute(&mut *tx)
            .await?;
        let row=sqlx::query("UPDATE claim_candidates c SET status='withdrawn',updated_at=now() WHERE id=$1 AND principal_key=$2 AND status IN ('waitlisted','exclusive','authorization_ready','withdrawn') AND claim_transaction_hash IS NULL AND canonical_event_id IS NULL AND (relay_guard_until IS NULL OR relay_guard_until<=now()) AND NOT EXISTS(SELECT 1 FROM bond_sponsorships b WHERE b.claim_candidate_id=c.id AND (b.status IN ('broadcast','confirmed') OR b.transaction_hash IS NOT NULL OR b.failure_code='broadcast_unknown')) AND NOT EXISTS(SELECT 1 FROM x402_relay_attempts r WHERE r.network=c.network AND r.bounty_contract=c.bounty_contract AND r.authorization_nonce=c.authorization_nonce AND r.status IN ('prepared','relaying','broadcast','confirmed')) RETURNING c.*")
            .bind(id).bind(principal).fetch_optional(&mut *tx).await?.ok_or_else(||DbError::ClaimCandidateConflict("candidate is not owned or a relay is pending".into()))?;
        let candidate = claim_candidate_from_row(row)?;
        sqlx::query("UPDATE bond_sponsorships SET status='failed',failure_code='reservation_withdrawn',failure_message='The owner withdrew before a relay attempt.',updated_at=now() WHERE claim_candidate_id=$1 AND status='reserved' AND transaction_hash IS NULL")
            .bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(candidate)
    }

    /// Serialize relay admission against withdrawal. Existing signed-request retries may continue.
    pub async fn begin_owned_claim_relay(&self, id: Uuid, principal: &str) -> DbResult<()> {
        let candidate = self
            .get_claim_candidate(id)
            .await?
            .ok_or_else(|| DbError::ClaimCandidateConflict("candidate not found".into()))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "claim-queue:{}:{}",
                candidate.network, candidate.bounty_contract
            ))
            .execute(&mut *tx)
            .await?;
        let row=sqlx::query("UPDATE claim_candidates SET relay_guard_until=now()+interval '2 minutes',updated_at=now() WHERE id=$1 AND principal_key=$2 AND status IN ('authorization_ready','relaying') AND exclusive_until>now() RETURNING id")
            .bind(id).bind(principal).fetch_optional(&mut *tx).await?;
        if row.is_none() {
            return Err(DbError::ClaimCandidateConflict(
                "claim reservation is not live".into(),
            ));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Refresh the candidate after acquiring the relayer lease and durably fence
    /// the only send-capable attempt. `true` authorizes this caller to send once;
    /// `false` returns an already pending/confirmed candidate for reconciliation.
    /// A crash or ambiguous RPC result leaves the no-hash relaying row intact.
    pub async fn admit_owned_claim_broadcast(
        &self,
        id: Uuid,
        nonce: &str,
        valid_before: u64,
        lease: Uuid,
    ) -> DbResult<(ClaimCandidate, bool)> {
        let snapshot = self
            .get_claim_candidate(id)
            .await?
            .ok_or_else(|| DbError::ClaimCandidateConflict("candidate not found".into()))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "claim-queue:{}:{}",
                snapshot.network, snapshot.bounty_contract
            ))
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT * FROM claim_candidates WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::ClaimCandidateConflict("candidate not found".into()))?;
        let candidate = claim_candidate_from_row(row)?;
        if candidate.authorization_nonce.as_deref() != Some(nonce)
            || candidate.authorization_valid_before != Some(valid_before)
        {
            return Err(DbError::ClaimCandidateConflict(
                "authorization scope changed before relay".into(),
            ));
        }
        if matches!(
            candidate.status,
            ClaimCandidateStatus::Relaying | ClaimCandidateStatus::Claimed
        ) {
            tx.commit().await?;
            return Ok((candidate, false));
        }
        let row = sqlx::query(r#"
            UPDATE claim_candidates SET status='relaying',failure_code='claim_broadcast_unknown',
              failure_message='The relay outcome needs canonical or operator reconciliation; do not broadcast again.',
              updated_at=now()
            WHERE id=$1 AND status='authorization_ready' AND principal_key IS NOT NULL
              AND claim_transaction_hash IS NULL AND canonical_event_id IS NULL
              AND exclusive_until>now() AND relay_guard_until>now()
              AND authorization_valid_before>=extract(epoch FROM now())+6
              AND EXISTS(SELECT 1 FROM x402_relayer_leases l WHERE l.network=claim_candidates.network
                AND l.lease_token=$2 AND l.lease_expires_at>now())
            RETURNING *
        "#).bind(id).bind(lease).fetch_optional(&mut *tx).await?
            .ok_or_else(|| DbError::ClaimCandidateConflict("claim reservation or authorization is no longer live".into()))?;
        let candidate = claim_candidate_from_row(row)?;
        tx.commit().await?;
        Ok((candidate, true))
    }

    /// Use only for a typed error known to precede broadcasting. Unknown provider
    /// results and timeouts must retain the pending marker, never call this method.
    pub async fn fail_owned_claim_before_broadcast(&self, id: Uuid) -> DbResult<ClaimCandidate> {
        let snapshot = self
            .get_claim_candidate(id)
            .await?
            .ok_or_else(|| DbError::ClaimCandidateConflict("candidate not found".into()))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "claim-queue:{}:{}",
                snapshot.network, snapshot.bounty_contract
            ))
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(
            r#"
            UPDATE claim_candidates SET status='failed',failure_code='claim_prebroadcast_failed',
              failure_message='The claim failed before broadcast; no transaction was sent.',
              relay_guard_until=NULL,updated_at=now()
            WHERE id=$1 AND status='relaying' AND failure_code='claim_broadcast_unknown'
              AND claim_transaction_hash IS NULL AND canonical_event_id IS NULL
            RETURNING *
        "#,
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict("candidate has a recorded or resolved relay".into())
        })?;
        let candidate = claim_candidate_from_row(row)?;
        sqlx::query("UPDATE bond_sponsorships SET status='failed',failure_code='claim_prebroadcast_failed',failure_message='The claim failed before broadcast; no transaction was sent.',updated_at=now() WHERE claim_candidate_id=$1 AND status='reserved' AND transaction_hash IS NULL")
            .bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(candidate)
    }
    pub async fn reserve_owned_claim_candidate(
        &self,
        candidate: &NewClaimCandidate,
        exclusive_seconds: u64,
        waitlist_capacity: u16,
        principal: &str,
        fingerprint: &str,
        admission_limit: i64,
    ) -> DbResult<ClaimCandidateReservation> {
        if !candidate.eligibility_decision.eligible {
            return Err(DbError::ClaimCandidateConflict(
                "ineligible candidates cannot enter the claim queue".to_string(),
            ));
        }
        if exclusive_seconds == 0 || waitlist_capacity == 0 {
            return Err(DbError::ClaimCandidateConflict(
                "claim queue bounds must be positive".to_string(),
            ));
        }
        let network = candidate.network.trim().to_ascii_lowercase();
        let bounty = normalize_key_address(&candidate.bounty_contract);
        let solver = normalize_key_address(&candidate.solver_wallet);
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("claim-queue:{network}:{bounty}"))
            .execute(&mut *transaction)
            .await?;

        let existing = sqlx::query(CLAIM_CANDIDATE_SELECT_BY_IDEMPOTENCY_SQL)
            .bind(&candidate.idempotency_key)
            .fetch_optional(&mut *transaction)
            .await?
            .map(claim_candidate_from_row)
            .transpose()?;
        if let Some(existing) = existing {
            let owner: (Option<String>, Option<String>) = sqlx::query_as(
                "SELECT principal_key,request_fingerprint FROM claim_candidates WHERE id=$1",
            )
            .bind(existing.id)
            .fetch_one(&mut *transaction)
            .await?;
            if owner.0.as_deref() != Some(principal)
                || owner.1.as_deref() != Some(fingerprint)
                || existing.network != network
                || existing.bounty_contract != bounty
                || existing.solver_wallet != solver
                || existing.agent_id != candidate.agent_id
            {
                return Err(DbError::ClaimCandidateConflict(
                    "idempotency key was already used for different claim inputs".to_string(),
                ));
            }
            let position = waitlist_position(&mut transaction, &existing).await?;
            transaction.commit().await?;
            return Ok(ClaimCandidateReservation {
                candidate: existing,
                waitlist_position: position,
            });
        }

        let quota_principal = format!("claim-wallet:{network}:{solver}");
        auth::consume_security_quota_in_transaction(
            &mut transaction,
            &quota_principal,
            "admission",
            &candidate.idempotency_key,
            fingerprint,
            admission_limit,
            60,
        )
        .await
        .map_err(|e| match e {
            auth::AuthStoreError::QuotaExceeded => DbError::ClaimAdmissionQuotaExceeded,
            auth::AuthStoreError::Unavailable(e) => DbError::Sqlx(e),
            _ => DbError::ClaimCandidateConflict(e.to_string()),
        })?;

        if sqlx::query(ACTIVE_CLAIM_CANDIDATE_SELECT_SQL)
            .bind(&network)
            .bind(&bounty)
            .bind(&solver)
            .fetch_optional(&mut *transaction)
            .await?
            .is_some()
        {
            return Err(DbError::ClaimCandidateConflict(
                "solver already has an active request for this bounty; replay its original idempotency key"
                    .to_string(),
            ));
        }

        let active_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM claim_candidates
            WHERE network = $1 AND bounty_contract = $2
              AND status IN ('exclusive', 'sponsoring', 'authorization_ready', 'relaying')
            "#,
        )
        .bind(&network)
        .bind(&bounty)
        .fetch_one(&mut *transaction)
        .await?;
        let status = if active_count == 0 {
            "exclusive"
        } else {
            let waitlisted: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM claim_candidates WHERE network = $1 AND bounty_contract = $2 AND status = 'waitlisted'",
            )
            .bind(&network)
            .bind(&bounty)
            .fetch_one(&mut *transaction)
            .await?;
            if waitlisted >= i64::from(waitlist_capacity) {
                return Err(DbError::ClaimWaitlistFull);
            }
            "waitlisted"
        };
        let exclusive_seconds = i64_from_u64(exclusive_seconds)?;
        let row = sqlx::query(
            r#"
            INSERT INTO claim_candidates
              (id, idempotency_key, network, bounty_contract, solver_wallet, agent_id,
               eligibility_evidence, eligibility_decision, status, exclusive_until, principal_key, request_fingerprint)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                    CASE WHEN $9 = 'exclusive'
                         THEN now() + make_interval(secs => $10) ELSE NULL END, $11, $12)
            RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                      agent_id, eligibility_evidence, eligibility_decision, status,
                      exclusive_until, authorization_nonce, authorization_valid_before,
                      claim_transaction_hash, canonical_event_id, failure_code,
                      failure_message, created_at, updated_at
            "#,
        )
        .bind(candidate.id)
        .bind(&candidate.idempotency_key)
        .bind(&network)
        .bind(&bounty)
        .bind(&solver)
        .bind(candidate.agent_id)
        .bind(serde_json::to_value(&candidate.eligibility_evidence)?)
        .bind(serde_json::to_value(&candidate.eligibility_decision)?)
        .bind(status)
        .bind(exclusive_seconds)
        .bind(principal)
        .bind(fingerprint)
        .fetch_one(&mut *transaction)
        .await?;
        let candidate = claim_candidate_from_row(row)?;
        let position = waitlist_position(&mut transaction, &candidate).await?;
        transaction.commit().await?;
        Ok(ClaimCandidateReservation {
            candidate,
            waitlist_position: position,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn digest(value: &str) -> String {
        hex::encode(Sha256::digest(value.as_bytes()))
    }
    fn address() -> String {
        let value = Uuid::new_v4().simple().to_string();
        format!("0x{value}{}", &value[..8])
    }
    fn input(network: &str, bounty: &str, wallet: &str) -> NewClaimCandidate {
        NewClaimCandidate {
            id: Uuid::new_v4(),
            idempotency_key: digest(&Uuid::new_v4().to_string()),
            network: network.into(),
            bounty_contract: bounty.into(),
            solver_wallet: wallet.into(),
            agent_id: None,
            eligibility_evidence: AgentEligibilityEvidence {
                agent_id: None,
                solver_wallet: wallet.into(),
                capabilities: vec![],
                paid_completions: 0,
                paid_usdc_base_units: 0,
            },
            eligibility_decision: AgentEligibilityDecision {
                eligible: true,
                reasons: vec![],
            },
        }
    }
    async fn fixture_store() -> PostgresStore {
        let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL")
            .expect("isolated test database required");
        crate::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        store
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn durable_queue_replays_expiry_promotion_and_pending_relay() {
        let store = fixture_store().await;
        let other = fixture_store().await;
        let network = format!("queue-test-{}", Uuid::new_v4());
        let bounty = address();
        let a = input(&network, &bounty, &address());
        let principal = format!("wallet:8453:{}", a.solver_wallet);
        let fp = digest("same inputs");
        let (first, second) = tokio::join!(
            store.reserve_owned_claim_candidate(&a, 900, 20, &principal, &fp, 10),
            other.reserve_owned_claim_candidate(&a, 900, 20, &principal, &fp, 10)
        );
        let first = first.unwrap().candidate;
        let second = second.unwrap().candidate;
        assert_eq!(first.id, second.id);
        assert_eq!(first.exclusive_until, second.exclusive_until);
        assert!(
            (first.exclusive_until.unwrap() - first.created_at - Duration::seconds(900))
                .num_seconds()
                .abs()
                <= 1
        );
        assert!(store
            .reserve_owned_claim_candidate(&a, 900, 20, "wallet:8453:wrong-owner", &fp, 10)
            .await
            .is_err());
        assert!(store
            .reserve_owned_claim_candidate(&a, 900, 20, &principal, &digest("different inputs"), 10)
            .await
            .is_err());
        let retry = input(&network, &bounty, &a.solver_wallet);
        assert!(store
            .reserve_owned_claim_candidate(&retry, 900, 20, &principal, &fp, 10)
            .await
            .is_err());
        let b = input(&network, &bounty, &address());
        let bp = format!("wallet:8453:{}", b.solver_wallet);
        let waiting = store
            .reserve_owned_claim_candidate(&b, 900, 20, &bp, &fp, 10)
            .await
            .unwrap()
            .candidate;
        assert_eq!(waiting.status, ClaimCandidateStatus::Waitlisted);
        sqlx::query(
            "UPDATE claim_candidates SET exclusive_until=now()-interval '1 second' WHERE id=$1",
        )
        .bind(a.id)
        .execute(&store.pool)
        .await
        .unwrap();
        // No eligible waiter is promoted. Processing still expires the old exclusive lease.
        store
            .maintain_owned_claim_queue(&network, &bounty, 900, &[])
            .await
            .unwrap();
        assert_eq!(
            store
                .get_claim_candidate(a.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Superseded
        );
        assert_eq!(
            store
                .get_claim_candidate(b.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Waitlisted
        );
        other
            .maintain_owned_claim_queue(&network, &bounty, 900, &[b.id])
            .await
            .unwrap();
        let promoted = store.get_claim_candidate(b.id).await.unwrap().unwrap();
        assert_eq!(promoted.status, ClaimCandidateStatus::Exclusive);
        assert!((promoted.exclusive_until.unwrap() - Utc::now()).num_seconds() >= 898);
        assert!(store
            .withdraw_owned_claim_candidate(b.id, &principal)
            .await
            .is_err());
        let valid_before = (Utc::now() + Duration::minutes(15)).timestamp() as u64;
        let nonce_a = format!("0x{}", "ab".repeat(32));
        let nonce_b = format!("0x{}", "ef".repeat(32));
        let (auth_a, auth_b) = tokio::join!(
            store.set_claim_candidate_authorization(b.id, &nonce_a, valid_before),
            other.set_claim_candidate_authorization(b.id, &nonce_b, valid_before + 60)
        );
        let auth_a = auth_a.unwrap();
        let auth_b = auth_b.unwrap();
        assert_eq!(auth_a.authorization_nonce, auth_b.authorization_nonce);
        assert_eq!(
            auth_a.authorization_valid_before,
            auth_b.authorization_valid_before
        );
        assert_eq!(auth_a.exclusive_until, auth_b.exclusive_until);
        store.begin_owned_claim_relay(b.id, &bp).await.unwrap();
        assert!(other
            .withdraw_owned_claim_candidate(b.id, &bp)
            .await
            .is_err());
        let tx_hash = format!("0x{}", "cd".repeat(32));
        store
            .mark_claim_candidate_relaying(b.id, &tx_hash)
            .await
            .unwrap();
        let c = input(&network, &bounty, &address());
        let cp = format!("wallet:8453:{}", c.solver_wallet);
        store
            .reserve_owned_claim_candidate(&c, 900, 20, &cp, &fp, 10)
            .await
            .unwrap();
        sqlx::query("UPDATE claim_candidates SET exclusive_until=now()-interval '1 day',relay_guard_until=now()-interval '1 hour' WHERE id=$1").bind(b.id).execute(&store.pool).await.unwrap();
        store
            .maintain_owned_claim_queue(&network, &bounty, 900, &[c.id])
            .await
            .unwrap();
        let pending = store.get_claim_candidate(b.id).await.unwrap().unwrap();
        assert_eq!(pending.status, ClaimCandidateStatus::Relaying);
        assert_eq!(pending.claim_transaction_hash, Some(tx_hash));
        assert_eq!(
            store
                .get_claim_candidate(c.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Waitlisted
        );
        sqlx::query("UPDATE claim_candidates SET created_at=now()-interval '25 hours' WHERE id=$1")
            .bind(c.id)
            .execute(&store.pool)
            .await
            .unwrap();
        other
            .maintain_owned_claim_queue(&network, &bounty, 900, &[c.id])
            .await
            .unwrap();
        assert_eq!(
            store
                .get_claim_candidate(c.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Superseded
        );
        // An ordinary owned cancellation is idempotent and allows the next eligible waiter.
        let d = input(&network, &address(), &address());
        let dp = format!("wallet:8453:{}", d.solver_wallet);
        store
            .reserve_owned_claim_candidate(&d, 900, 20, &dp, &fp, 10)
            .await
            .unwrap();
        assert_eq!(
            store
                .withdraw_owned_claim_candidate(d.id, &dp)
                .await
                .unwrap()
                .status,
            ClaimCandidateStatus::Withdrawn
        );
        assert_eq!(
            other
                .withdraw_owned_claim_candidate(d.id, &dp)
                .await
                .unwrap()
                .status,
            ClaimCandidateStatus::Withdrawn
        );
    }

    async fn sponsor(store: &PostgresStore, candidate: &NewClaimCandidate) -> BondSponsorship {
        store
            .reserve_bond_sponsorship(
                &NewBondSponsorship {
                    id: Uuid::new_v4(),
                    claim_candidate_id: candidate.id,
                    network: candidate.network.clone(),
                    bounty_contract: candidate.bounty_contract.clone(),
                    solver_wallet: candidate.solver_wallet.clone(),
                    sponsor_wallet: address(),
                    amount: 100,
                },
                10_000,
                1_000,
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn durable_queue_unused_sponsorship_expires_and_withdraws() {
        let store = fixture_store().await;
        let network = format!("unused-sponsor-{}", Uuid::new_v4());
        let bounty = address();
        let a = input(&network, &bounty, &address());
        let owner = format!("wallet:8453:{}", a.solver_wallet);
        let fp = digest("owned fixture");
        store
            .reserve_owned_claim_candidate(&a, 900, 20, &owner, &fp, 10)
            .await
            .unwrap();
        store
            .set_claim_candidate_authorization(a.id, &digest("nonce"), 1300)
            .await
            .unwrap();
        store.begin_owned_claim_relay(a.id, &owner).await.unwrap();
        sponsor(&store, &a).await;
        let b = input(&network, &bounty, &address());
        store
            .reserve_owned_claim_candidate(
                &b,
                900,
                20,
                &format!("wallet:8453:{}", b.solver_wallet),
                &fp,
                10,
            )
            .await
            .unwrap();
        sqlx::query("UPDATE claim_candidates SET exclusive_until=now()-interval '2 days',relay_guard_until=now()-interval '2 days' WHERE id=$1")
            .bind(a.id).execute(&store.pool).await.unwrap();
        store
            .maintain_owned_claim_queue(&network, &bounty, 900, &[b.id])
            .await
            .unwrap();
        assert_eq!(
            store
                .get_claim_candidate(a.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Superseded
        );
        assert_eq!(
            store
                .get_claim_candidate(b.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Exclusive
        );
        let expired = store
            .get_bond_sponsorship_for_candidate(a.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expired.status, BondSponsorshipStatus::Failed);
        assert_eq!(expired.failure_code.as_deref(), Some("reservation_expired"));

        let c = input(&network, &address(), &address());
        let owner = format!("wallet:8453:{}", c.solver_wallet);
        store
            .reserve_owned_claim_candidate(&c, 900, 20, &owner, &fp, 10)
            .await
            .unwrap();
        sponsor(&store, &c).await;
        let withdrawn = store
            .withdraw_owned_claim_candidate(c.id, &owner)
            .await
            .unwrap();
        assert_eq!(withdrawn.status, ClaimCandidateStatus::Withdrawn);
        assert_eq!(
            store
                .get_bond_sponsorship_for_candidate(c.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            BondSponsorshipStatus::Failed
        );

        // Historical sponsorships have no owned-attempt marker contract. Do not
        // reinterpret their missing hash as proof that broadcasting never began.
        let legacy = input(&network, &address(), &address());
        store
            .reserve_owned_claim_candidate(&legacy, 900, 20, "synthetic-legacy", &fp, 10)
            .await
            .unwrap();
        sponsor(&store, &legacy).await;
        sqlx::query("UPDATE claim_candidates SET principal_key=NULL,exclusive_until=now()-interval '2 days' WHERE id=$1")
            .bind(legacy.id).execute(&store.pool).await.unwrap();
        store
            .maintain_owned_claim_queue(&network, &legacy.bounty_contract, 900, &[])
            .await
            .unwrap();
        assert_eq!(
            store
                .get_claim_candidate(legacy.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Exclusive
        );
        assert_eq!(
            store
                .get_bond_sponsorship_for_candidate(legacy.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            BondSponsorshipStatus::Reserved
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn durable_queue_broadcast_once_preserves_unknown_and_releases_known_failure() {
        let store = fixture_store().await;
        let other = fixture_store().await;
        let network = format!("broadcast-once-{}", Uuid::new_v4());
        let bounty = address();
        let a = input(&network, &bounty, &address());
        let owner = format!("wallet:8453:{}", a.solver_wallet);
        let fp = digest("exact signed request");
        let nonce = format!("0x{}", digest("nonce"));
        let deadline = (Utc::now() + Duration::minutes(10)).timestamp() as u64;
        store
            .reserve_owned_claim_candidate(&a, 900, 20, &owner, &fp, 10)
            .await
            .unwrap();
        store
            .set_claim_candidate_authorization(a.id, &nonce, deadline)
            .await
            .unwrap();
        store.begin_owned_claim_relay(a.id, &owner).await.unwrap();
        let sponsorship = sponsor(&store, &a).await;
        let lease = store
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .admit_owned_claim_broadcast(a.id, &nonce, deadline, Uuid::new_v4())
            .await
            .is_err());
        assert!(store
            .admit_owned_claim_broadcast(a.id, "wrong nonce", deadline, lease)
            .await
            .is_err());
        assert_eq!(
            store
                .get_claim_candidate(a.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::AuthorizationReady
        );
        let (first, second) = tokio::join!(
            store.admit_owned_claim_broadcast(a.id, &nonce, deadline, lease),
            other.admit_owned_claim_broadcast(a.id, &nonce, deadline, lease),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(
            u8::from(first.1) + u8::from(second.1),
            1,
            "exactly one process may send"
        );
        assert_eq!(first.0.id, second.0.id);
        assert_eq!(first.0.status, ClaimCandidateStatus::Relaying);
        assert!(first.0.claim_transaction_hash.is_none());
        assert_eq!(
            first.0.failure_code.as_deref(),
            Some("claim_broadcast_unknown")
        );
        store
            .release_x402_relayer_lease(&network, lease)
            .await
            .unwrap();
        let b = input(&network, &bounty, &address());
        store
            .reserve_owned_claim_candidate(
                &b,
                900,
                20,
                &format!("wallet:8453:{}", b.solver_wallet),
                &fp,
                10,
            )
            .await
            .unwrap();
        sqlx::query("UPDATE claim_candidates SET exclusive_until=now()-interval '2 days',authorization_valid_before=1300,relay_guard_until=now()-interval '2 days' WHERE id=$1")
            .bind(a.id).execute(&store.pool).await.unwrap();
        other
            .maintain_owned_claim_queue(&network, &bounty, 900, &[b.id])
            .await
            .unwrap();
        assert!(store
            .withdraw_owned_claim_candidate(a.id, &owner)
            .await
            .is_err());
        assert_eq!(
            store
                .get_claim_candidate(b.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Waitlisted
        );
        assert_eq!(
            store
                .get_bond_sponsorship_for_candidate(a.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            BondSponsorshipStatus::Reserved
        );
        let lease = other
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .unwrap();
        let expired_retry = other
            .admit_owned_claim_broadcast(a.id, &nonce, 1300, lease)
            .await
            .unwrap();
        assert!(
            !expired_retry.1,
            "expired unknown request is recoverable, never rebroadcastable"
        );
        let hash = format!("0x{}", digest("local transaction"));
        store
            .mark_atomic_sponsored_claim_broadcast(a.id, sponsorship.id, &hash)
            .await
            .unwrap();
        let fresh = other
            .admit_owned_claim_broadcast(a.id, &nonce, 1300, lease)
            .await
            .unwrap();
        assert!(!fresh.1);
        assert_eq!(
            fresh.0.claim_transaction_hash.as_deref(),
            Some(hash.as_str())
        );
        assert!(fresh.0.failure_code.is_none());
        assert!(
            other.fail_owned_claim_before_broadcast(a.id).await.is_err(),
            "pending hash must never be discarded"
        );
        other
            .release_x402_relayer_lease(&network, lease)
            .await
            .unwrap();

        let c = input(&network, &address(), &address());
        let cp = format!("wallet:8453:{}", c.solver_wallet);
        store
            .reserve_owned_claim_candidate(&c, 900, 20, &cp, &fp, 10)
            .await
            .unwrap();
        store
            .set_claim_candidate_authorization(c.id, &nonce, deadline)
            .await
            .unwrap();
        store.begin_owned_claim_relay(c.id, &cp).await.unwrap();
        sponsor(&store, &c).await;
        let lease = store
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .admit_owned_claim_broadcast(c.id, &nonce, deadline, lease)
                .await
                .unwrap()
                .1
        );
        store.fail_owned_claim_before_broadcast(c.id).await.unwrap();
        store
            .release_x402_relayer_lease(&network, lease)
            .await
            .unwrap();
        assert_eq!(
            store
                .get_claim_candidate(c.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ClaimCandidateStatus::Failed
        );
        assert_eq!(
            store
                .get_bond_sponsorship_for_candidate(c.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            BondSponsorshipStatus::Failed
        );
        let next = input(&network, &c.bounty_contract, &address());
        assert_eq!(
            store
                .reserve_owned_claim_candidate(
                    &next,
                    900,
                    20,
                    &format!("wallet:8453:{}", next.solver_wallet),
                    &fp,
                    10
                )
                .await
                .unwrap()
                .candidate
                .status,
            ClaimCandidateStatus::Exclusive
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn durable_queue_quota_and_bounded_maintenance() {
        let store = fixture_store().await;
        let other = fixture_store().await;
        let network = format!("queue-quota-{}", Uuid::new_v4());
        let wallet = address();
        let principal = format!("wallet:8453:{wallet}");
        let fp = digest("request");
        for i in 0..10 {
            let a = input(&network, &address(), &wallet);
            let saved = store
                .reserve_owned_claim_candidate(&a, 600, 20, &principal, &fp, 10)
                .await
                .unwrap();
            assert_eq!(saved.candidate.status, ClaimCandidateStatus::Exclusive);
            assert_eq!(
                other
                    .reserve_owned_claim_candidate(&a, 600, 20, &principal, &fp, 10)
                    .await
                    .unwrap()
                    .candidate
                    .id,
                a.id
            );
            assert!(i < 10);
        }
        assert!(matches!(
            store
                .reserve_owned_claim_candidate(
                    &input(&network, &address(), &wallet),
                    600,
                    20,
                    &principal,
                    &fp,
                    10
                )
                .await,
            Err(DbError::ClaimAdmissionQuotaExceeded)
        ));
        let (a, b) = tokio::join!(
            store.claim_maintenance_batch(),
            other.claim_maintenance_batch()
        );
        assert!(a.unwrap().len() <= 100);
        assert!(b.unwrap().len() <= 100);
    }
}
