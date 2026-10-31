//! The review of each canonical round of a funded card bounty. An acceptance is signed when it
//! is recorded. A rejection waits unsigned for the solver's appeal: the review key signs it when
//! the appeal period closes unused, or signs the arbiter's ruling instead. Every write happens
//! inside the order-locked transaction, and each round releases at most one signature, so a
//! retry returns the recorded outcome and a different one is refused.
use super::{stripe::locked_order, ManagedOrderStatus, ManagedOrderStoreError, Result};
use crate::PostgresStore;
use managed_orders::review::{AppealRuling, CustomerVerdict, ReviewAppeal};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

/// Who recorded the verdict: the customer, or the platform after the customer stayed silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecider {
    Customer,
    Deadline,
}

impl ReviewDecider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Customer => "customer",
            Self::Deadline => "deadline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAppeal {
    pub appeal: ReviewAppeal,
    pub appeal_hash: String,
    /// The solver wallet's EIP-712 `ReviewAppeal` signature.
    pub signature: String,
    pub appealed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRuling {
    pub ruling: AppealRuling,
    pub response_hash: String,
    pub ruled_by: String,
    pub ruled_at: i64,
}

/// The one attestation the review key released for the round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedOutcome {
    pub passed: bool,
    pub response_hash: String,
    pub verifier: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCustomerReview {
    pub order_id: Uuid,
    pub verdict: CustomerVerdict,
    /// The response hash of the verdict document.
    pub response_hash: String,
    pub decided_by: ReviewDecider,
    /// The submission's verification expiry, which is also the attestation deadline.
    pub deadline: u64,
    pub recorded_at: i64,
    /// A rejection's appeal period end, in unix seconds.
    pub appeal_ends_at: Option<u64>,
    pub appeal: Option<RecordedAppeal>,
    pub ruling: Option<RecordedRuling>,
    pub signed: Option<SignedOutcome>,
}

impl ManagedCustomerReview {
    /// The public state of a rejection's appeal at `now`; `None` for an acceptance.
    pub fn appeal_status(&self, now: u64) -> Option<&'static str> {
        if self.verdict.passed {
            return None;
        }
        Some(match (&self.ruling, &self.appeal, self.appeal_ends_at) {
            (Some(ruling), _, _) if ruling.ruling.passed => "reversed",
            (Some(_), _, _) => "upheld",
            (None, Some(_), _) => "appealed",
            (None, None, Some(ends)) if self.signed.is_none() && now < ends => "window_open",
            _ => "final",
        })
    }

    /// Whether the arbiter reversed this round's rejection.
    pub fn reversed(&self) -> bool {
        self.ruling
            .as_ref()
            .is_some_and(|ruling| ruling.ruling.passed)
    }
}

/// The review key's signature over the attestation for one outcome and deadline.
pub struct ReviewSignature {
    pub verifier: String,
    pub signature: String,
}

/// Signs `(passed, response_hash, deadline)` as the bounty's v2 verification attestation.
pub trait ReviewSigner:
    FnOnce(bool, &str, u64) -> std::result::Result<ReviewSignature, ()>
{
}
impl<F: FnOnce(bool, &str, u64) -> std::result::Result<ReviewSignature, ()>> ReviewSigner for F {}

fn integrity<T>(_: T) -> ManagedOrderStoreError {
    ManagedOrderStoreError::Integrity
}

fn conflict() -> ManagedOrderStoreError {
    ManagedOrderStoreError::Quote(managed_orders::OrderError::Conflict)
}

fn unix(row: &sqlx::postgres::PgRow, column: &str) -> Result<Option<i64>> {
    Ok(row
        .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(column)?
        .map(|at| at.timestamp()))
}

fn review_from_row(row: &sqlx::postgres::PgRow) -> Result<ManagedCustomerReview> {
    let verdict: CustomerVerdict =
        serde_json::from_value(row.try_get("verdict")?).map_err(integrity)?;
    let response_hash: String = row.try_get("response_hash")?;
    if verdict.response_hash().ok().as_deref() != Some(response_hash.as_str())
        || i64::try_from(verdict.round).ok() != Some(row.try_get::<i64, _>("round")?)
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    let decided_by = match row.try_get::<String, _>("decided_by")?.as_str() {
        "customer" => ReviewDecider::Customer,
        "deadline" => ReviewDecider::Deadline,
        _ => return Err(ManagedOrderStoreError::Integrity),
    };
    let appeal = match row.try_get::<Option<serde_json::Value>, _>("appeal")? {
        None => None,
        Some(value) => {
            let appeal: ReviewAppeal = serde_json::from_value(value).map_err(integrity)?;
            let appeal_hash: String = row.try_get("appeal_hash")?;
            if appeal.appeal_hash().ok().as_deref() != Some(appeal_hash.as_str())
                || !appeal.appeals(&verdict, &response_hash)
            {
                return Err(ManagedOrderStoreError::Integrity);
            }
            Some(RecordedAppeal {
                appeal,
                appeal_hash,
                signature: row.try_get("appeal_signature")?,
                appealed_at: unix(row, "appealed_at")?.ok_or(ManagedOrderStoreError::Integrity)?,
            })
        }
    };
    let ruling = match row.try_get::<Option<serde_json::Value>, _>("ruling")? {
        None => None,
        Some(value) => {
            let ruling: AppealRuling = serde_json::from_value(value).map_err(integrity)?;
            let recorded = appeal.as_ref().ok_or(ManagedOrderStoreError::Integrity)?;
            let expected = AppealRuling::on(
                &verdict,
                &recorded.appeal,
                ruling.passed,
                ruling.reason.clone(),
            )
            .map_err(integrity)?;
            if expected != ruling {
                return Err(ManagedOrderStoreError::Integrity);
            }
            Some(RecordedRuling {
                response_hash: ruling.response_hash().map_err(integrity)?,
                ruling,
                ruled_by: row.try_get("ruled_by")?,
                ruled_at: unix(row, "ruled_at")?.ok_or(ManagedOrderStoreError::Integrity)?,
            })
        }
    };
    let signed = match row.try_get::<Option<String>, _>("signature")? {
        None => None,
        Some(signature) => {
            let outcome = SignedOutcome {
                passed: row
                    .try_get::<Option<bool>, _>("signed_passed")?
                    .ok_or(ManagedOrderStoreError::Integrity)?,
                response_hash: row
                    .try_get::<Option<String>, _>("signed_response_hash")?
                    .ok_or(ManagedOrderStoreError::Integrity)?,
                verifier: row
                    .try_get::<Option<String>, _>("verifier")?
                    .ok_or(ManagedOrderStoreError::Integrity)?,
                signature,
            };
            // The signature commits to the ruling when there is one, else to the verdict.
            let (passed, hash) = match &ruling {
                Some(ruling) => (ruling.ruling.passed, &ruling.response_hash),
                None => (verdict.passed, &response_hash),
            };
            if outcome.passed != passed || &outcome.response_hash != hash {
                return Err(ManagedOrderStoreError::Integrity);
            }
            Some(outcome)
        }
    };
    Ok(ManagedCustomerReview {
        order_id: row.try_get("order_id")?,
        verdict,
        response_hash,
        decided_by,
        deadline: u64::try_from(row.try_get::<i64, _>("deadline")?).map_err(integrity)?,
        recorded_at: unix(row, "recorded_at")?.ok_or(ManagedOrderStoreError::Integrity)?,
        appeal_ends_at: row
            .try_get::<Option<i64>, _>("appeal_ends_at")?
            .map(u64::try_from)
            .transpose()
            .map_err(integrity)?,
        appeal,
        ruling,
        signed,
    })
}

async fn locked_review(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    round: u64,
) -> Result<Option<ManagedCustomerReview>> {
    sqlx::query("SELECT * FROM managed_customer_reviews WHERE order_id=$1 AND round=$2")
        .bind(order_id)
        .bind(i64::try_from(round).map_err(integrity)?)
        .fetch_optional(&mut **tx)
        .await?
        .as_ref()
        .map(review_from_row)
        .transpose()
}

/// Signs the round's own verdict and stores the signature on its row.
async fn store_signature(
    tx: &mut Transaction<'_, Postgres>,
    review: &ManagedCustomerReview,
    sign: impl ReviewSigner,
) -> Result<ManagedCustomerReview> {
    let (passed, response_hash) = (review.verdict.passed, &review.response_hash);
    let signed = sign(passed, response_hash, review.deadline).map_err(integrity)?;
    let row = sqlx::query(
        "UPDATE managed_customer_reviews
            SET verifier=lower($3), signature=lower($4), signed_passed=$5,
                signed_response_hash=$6, signed_at=clock_timestamp()
          WHERE order_id=$1 AND round=$2 AND signature IS NULL RETURNING *",
    )
    .bind(review.order_id)
    .bind(i64::try_from(review.verdict.round).map_err(integrity)?)
    .bind(&signed.verifier)
    .bind(&signed.signature)
    .bind(passed)
    .bind(response_hash)
    .fetch_one(&mut **tx)
    .await?;
    review_from_row(&row)
}

impl PostgresStore {
    /// Record the buyer's verdict for a funded order's committed bounty. An acceptance is signed
    /// now. A rejection is stored unsigned with its appeal period end. The caller has checked the
    /// verdict against the indexed canonical submission and the decision deadline.
    pub async fn record_managed_customer_review(
        &self,
        buyer: &str,
        order_id: Uuid,
        verdict: &CustomerVerdict,
        deadline: u64,
        appeal_ends_at: Option<u64>,
        sign: impl ReviewSigner,
    ) -> Result<ManagedCustomerReview> {
        self.insert_review(
            Some(buyer),
            order_id,
            verdict,
            deadline,
            appeal_ends_at,
            sign,
        )
        .await
    }

    /// Record and sign the platform's acceptance of a round the customer left undecided. A
    /// round that already has a verdict is returned unchanged.
    pub async fn record_managed_deadline_acceptance(
        &self,
        order_id: Uuid,
        verdict: &CustomerVerdict,
        deadline: u64,
        sign: impl ReviewSigner,
    ) -> Result<ManagedCustomerReview> {
        if !verdict.passed {
            return Err(conflict());
        }
        self.insert_review(None, order_id, verdict, deadline, None, sign)
            .await
    }

    async fn insert_review(
        &self,
        buyer: Option<&str>,
        order_id: Uuid,
        verdict: &CustomerVerdict,
        deadline: u64,
        appeal_ends_at: Option<u64>,
        sign: impl ReviewSigner,
    ) -> Result<ManagedCustomerReview> {
        let response_hash = verdict.response_hash().map_err(|_| conflict())?;
        if verdict.passed == appeal_ends_at.is_some() {
            return Err(conflict());
        }
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        if buyer.is_some_and(|buyer| order.quote.input().buyer_principal != buyer) {
            return Err(ManagedOrderStoreError::NotFound);
        }
        if let Some(existing) = locked_review(&mut tx, order_id, verdict.round).await? {
            tx.commit().await?;
            return match buyer {
                None => Ok(existing),
                Some(_) if &existing.verdict == verdict => Ok(existing),
                Some(_) => Err(conflict()),
            };
        }
        super::recovered::reject_reserved(&mut tx, order_id).await?;
        if order.status != ManagedOrderStatus::Funded
            || verdict.bounty_contract != order.quote.input().bounty.predicted_bounty
        {
            return Err(conflict());
        }
        let decided_by = if buyer.is_some() {
            ReviewDecider::Customer
        } else {
            ReviewDecider::Deadline
        };
        let row = sqlx::query(
            "INSERT INTO managed_customer_reviews
               (order_id, round, verdict, response_hash, deadline, decided_by, appeal_ends_at,
                verifier, signature, signed_passed, signed_response_hash, signed_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,lower($8),lower($9),$10,$11,
                     CASE WHEN $9::text IS NULL THEN NULL ELSE clock_timestamp() END)
             RETURNING *",
        );
        let signed = if verdict.passed {
            Some(sign(true, &response_hash, deadline).map_err(integrity)?)
        } else {
            None
        };
        let row = row
            .bind(order_id)
            .bind(i64::try_from(verdict.round).map_err(integrity)?)
            .bind(serde_json::to_value(verdict).map_err(integrity)?)
            .bind(&response_hash)
            .bind(i64::try_from(deadline).map_err(integrity)?)
            .bind(decided_by.as_str())
            .bind(
                appeal_ends_at
                    .map(i64::try_from)
                    .transpose()
                    .map_err(integrity)?,
            )
            .bind(signed.as_ref().map(|s| s.verifier.clone()))
            .bind(signed.as_ref().map(|s| s.signature.clone()))
            .bind(signed.as_ref().map(|_| true))
            .bind(signed.as_ref().map(|_| response_hash.clone()))
            .fetch_one(&mut *tx)
            .await?;
        let review = review_from_row(&row)?;
        tx.commit().await?;
        Ok(review)
    }

    /// Record the solver's signed appeal of an unsigned rejection inside its appeal period. The
    /// caller has verified the solver's signature and that the solver made the submission.
    pub async fn record_managed_review_appeal(
        &self,
        order_id: Uuid,
        appeal: &ReviewAppeal,
        signature: &str,
        now: u64,
    ) -> Result<ManagedCustomerReview> {
        let appeal_hash = appeal.appeal_hash().map_err(|_| conflict())?;
        let mut tx = self.pool.begin().await?;
        locked_order(&mut tx, order_id).await?;
        let review = locked_review(&mut tx, order_id, appeal.round)
            .await?
            .ok_or(ManagedOrderStoreError::NotFound)?;
        if let Some(existing) = &review.appeal {
            tx.commit().await?;
            return if existing.appeal_hash == appeal_hash {
                Ok(review)
            } else {
                Err(conflict())
            };
        }
        super::recovered::reject_reserved(&mut tx, order_id).await?;
        if !appeal.appeals(&review.verdict, &review.response_hash)
            || review.signed.is_some()
            || review.appeal_ends_at.is_none_or(|ends| now >= ends)
        {
            return Err(conflict());
        }
        let row = sqlx::query(
            "UPDATE managed_customer_reviews
                SET appeal=$3, appeal_hash=$4, appeal_signature=lower($5), appealed_at=clock_timestamp()
              WHERE order_id=$1 AND round=$2 AND appeal IS NULL AND signature IS NULL RETURNING *",
        )
        .bind(order_id)
        .bind(i64::try_from(appeal.round).map_err(integrity)?)
        .bind(serde_json::to_value(appeal).map_err(integrity)?)
        .bind(&appeal_hash)
        .bind(signature)
        .fetch_one(&mut *tx)
        .await?;
        let review = review_from_row(&row)?;
        tx.commit().await?;
        Ok(review)
    }

    /// Sign a rejection whose appeal period closed without an appeal. Returns the round as
    /// recorded when it is already signed.
    pub async fn finalize_managed_rejection(
        &self,
        order_id: Uuid,
        round: u64,
        now: u64,
        sign: impl ReviewSigner,
    ) -> Result<ManagedCustomerReview> {
        let mut tx = self.pool.begin().await?;
        locked_order(&mut tx, order_id).await?;
        let review = locked_review(&mut tx, order_id, round)
            .await?
            .ok_or(ManagedOrderStoreError::NotFound)?;
        if review.signed.is_some() {
            tx.commit().await?;
            return Ok(review);
        }
        super::recovered::reject_reserved(&mut tx, order_id).await?;
        if review.verdict.passed
            || review.appeal.is_some()
            || review.appeal_ends_at.is_none_or(|ends| now < ends)
        {
            return Err(conflict());
        }
        let review = store_signature(&mut tx, &review, sign).await?;
        tx.commit().await?;
        Ok(review)
    }

    /// Record the arbiter's ruling on an appealed rejection and sign it. Repeating the same
    /// ruling returns it, and a different one is refused.
    pub async fn record_managed_review_ruling(
        &self,
        order_id: Uuid,
        round: u64,
        reversed: bool,
        reason: &str,
        ruled_by: &str,
        sign: impl ReviewSigner,
    ) -> Result<ManagedCustomerReview> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        let review = locked_review(&mut tx, order_id, round)
            .await?
            .ok_or(ManagedOrderStoreError::NotFound)?;
        let appeal = review.appeal.as_ref().ok_or_else(conflict)?;
        let ruling = AppealRuling::on(&review.verdict, &appeal.appeal, reversed, reason.to_owned())
            .map_err(|_| conflict())?;
        if let Some(existing) = &review.ruling {
            tx.commit().await?;
            return if existing.ruling == ruling {
                Ok(review)
            } else {
                Err(conflict())
            };
        }
        if review.signed.is_some() {
            return Err(ManagedOrderStoreError::Integrity);
        }
        super::recovered::reject_reserved(&mut tx, order_id).await?;
        // The ruling and its signature are written together: the row check requires both.
        let hash = ruling.response_hash().map_err(integrity)?;
        let signed = sign(reversed, &hash, review.deadline).map_err(integrity)?;
        let row = sqlx::query(
            "UPDATE managed_customer_reviews
                SET ruling=$3, ruled_by=$4, ruled_at=clock_timestamp(), verifier=lower($5),
                    signature=lower($6), signed_passed=$7, signed_response_hash=$8,
                    signed_at=clock_timestamp()
              WHERE order_id=$1 AND round=$2 AND ruling IS NULL AND signature IS NULL RETURNING *",
        )
        .bind(order_id)
        .bind(i64::try_from(round).map_err(integrity)?)
        .bind(serde_json::to_value(&ruling).map_err(integrity)?)
        .bind(ruled_by)
        .bind(&signed.verifier)
        .bind(&signed.signature)
        .bind(reversed)
        .bind(&hash)
        .fetch_one(&mut *tx)
        .await?;
        let review = review_from_row(&row)?;
        // A reversal owes the customer's review deposit to the solver, recorded with the ruling.
        let input = order.quote.input();
        if reversed
            && input
                .additional_costs
                .iter()
                .any(|cost| cost.code == managed_orders::pricing::REVIEW_DEPOSIT_CODE)
        {
            let amount =
                managed_orders::review::review_deposit_micro_usdc(input.solver_reward_micro_usdc);
            sqlx::query(
                "INSERT INTO managed_deposit_payouts(order_id, round, network, solver, amount_micro_usdc)
                 VALUES ($1,$2,$3,lower($4),$5)",
            )
            .bind(order_id)
            .bind(i64::try_from(round).map_err(integrity)?)
            .bind(&input.bounty.network)
            .bind(&appeal.appeal.solver)
            .bind(i64::try_from(amount).map_err(integrity)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(review)
    }

    /// The buyer's recorded verdicts, latest round first.
    pub async fn list_managed_customer_reviews(
        &self,
        buyer: &str,
        order_id: Uuid,
    ) -> Result<Vec<ManagedCustomerReview>> {
        let rows = sqlx::query(
            "SELECT r.* FROM managed_customer_reviews r
             JOIN managed_service_orders o ON o.order_id=r.order_id
             WHERE r.order_id=$1 AND o.buyer_principal=$2 ORDER BY r.round DESC",
        )
        .bind(order_id)
        .bind(buyer)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(review_from_row).collect()
    }

    /// The card order funding a committed bounty and its reviews, latest round first.
    pub async fn managed_reviews_for_bounty(
        &self,
        network: &str,
        bounty_contract: &str,
    ) -> Result<Option<(Uuid, Vec<ManagedCustomerReview>)>> {
        let Some(order_id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT order_id FROM managed_bounty_funding_operations
              WHERE plan->>'network'=$1 AND plan->>'predicted_bounty'=lower($2) AND outcome='funded'",
        )
        .bind(network)
        .bind(bounty_contract)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let rows = sqlx::query(
            "SELECT * FROM managed_customer_reviews WHERE order_id=$1 ORDER BY round DESC",
        )
        .bind(order_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some((
            order_id,
            rows.iter().map(review_from_row).collect::<Result<_>>()?,
        )))
    }

    /// Claim a bounded, durable rotation of funded card bounties for review or settlement retry.
    /// Stamp attempts before processing, so failures and restarts cannot starve newer work.
    /// Cooldown prevents concurrent processes immediately revisiting the same batch.
    /// Forfeiting a review deposit does not settle the solver's signed acceptance.
    pub async fn claim_managed_funded_bounties(
        &self,
        network: &str,
        limit: i64,
        retry_after_seconds: i64,
    ) -> Result<Vec<(Uuid, String)>> {
        let rows = sqlx::query(
            "WITH candidates AS (
             SELECT f.order_id, f.last_review_attempt_at
               FROM managed_bounty_funding_operations f
              WHERE f.outcome='funded' AND f.plan->>'network'=$1
                AND (f.last_review_attempt_at IS NULL
                  OR f.last_review_attempt_at <= statement_timestamp() - make_interval(secs => $3))
                AND NOT EXISTS (SELECT 1 FROM managed_deposit_refund_operations d WHERE d.order_id=f.order_id)
                AND (NOT EXISTS (SELECT 1 FROM managed_deposit_payouts p WHERE p.order_id=f.order_id)
                  OR NOT EXISTS (SELECT 1 FROM autonomous_bounty_events e
                    WHERE e.network=f.plan->>'network'
                      AND e.contract_address=lower(f.plan->>'predicted_bounty')
                      AND e.kind='bounty_settled' AND e.block_time_verified))
              ORDER BY f.last_review_attempt_at NULLS FIRST, f.order_id
              LIMIT $2 FOR UPDATE OF f SKIP LOCKED
             ), claimed AS (
               UPDATE managed_bounty_funding_operations f
               SET last_review_attempt_at=statement_timestamp()
               FROM candidates c WHERE f.order_id=c.order_id
               RETURNING f.order_id, f.plan->>'predicted_bounty' AS bounty
             ) SELECT claimed.order_id, claimed.bounty FROM claimed
               JOIN candidates c USING (order_id)
               ORDER BY c.last_review_attempt_at NULLS FIRST, c.order_id",
        )
        .bind(network)
        .bind(limit.clamp(1, 1_000))
        .bind(retry_after_seconds.max(0) as f64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("order_id")?, row.try_get("bounty")?)))
            .collect()
    }

    /// Appealed rejections awaiting the arbiter with their network, earliest expiry first.
    pub async fn list_open_managed_appeals(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, ManagedCustomerReview)>> {
        let rows = sqlx::query(
            "SELECT r.*, f.plan->>'network' AS network FROM managed_customer_reviews r
               JOIN managed_bounty_funding_operations f ON f.order_id=r.order_id
              WHERE r.appeal IS NOT NULL AND r.ruling IS NULL
              ORDER BY r.deadline, r.order_id LIMIT $1",
        )
        .bind(limit.clamp(1, 200))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("network")?, review_from_row(row)?)))
            .collect()
    }
}

#[cfg(test)]
pub(super) mod tests;
