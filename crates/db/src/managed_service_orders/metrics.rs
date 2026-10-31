//! Aggregate-only reporting. Receipts and signed refund events remain the authority.
use super::*;
use chrono::{DateTime, Utc};
use managed_orders::{FiatCurrency, PriceBasis};
use serde_json::{json, Value};

const LIMIT: usize = 5_000;
const BOUNDARY: &str = "Live managed card payments only, including live launch tests. USD equivalents use each accepted quote's offered price basis and $1/USDC face value, not executed FX or bank settlement. Receipts and refunds are separate from canonical bounty payouts and GMV. Platform fees are collected quoted fees, not profit or total platform revenue.";

#[derive(Default)]
struct Totals {
    paid: u64,
    refunds: u64,
    gross: i128,
    returned: i128,
    fees: i128,
}

fn usd_micro(amount: u64, currency: FiatCurrency, basis: &PriceBasis) -> Result<i128> {
    // Actual USD cents require no quote FX conversion. MXN retains its original offered basis.
    let (numerator, denominator) = match currency {
        FiatCurrency::Usd => (100, 1),
        FiatCurrency::Mxn => (
            basis.minor_units_per_usdc_numerator,
            basis.minor_units_per_usdc_denominator,
        ),
    };
    if numerator == 0 || denominator == 0 {
        return Err(ManagedOrderStoreError::Integrity);
    }
    let scaled = u128::from(amount)
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_mul(u128::from(denominator)))
        .ok_or(ManagedOrderStoreError::Integrity)?;
    i128::try_from(scaled / u128::from(numerator)).map_err(|_| ManagedOrderStoreError::Integrity)
}

fn decimal(value: i128) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let value = value.unsigned_abs();
    format!("{sign}{}.{:06}", value / 1_000_000, value % 1_000_000)
}

impl Totals {
    fn response(self, start: DateTime<Utc>, end: DateTime<Utc>) -> Value {
        json!({"schema_version":"agent-bounties/card-payment-metrics-v1", "status":"ready",
            "currency":"USD", "started_at":start.to_rfc3339(), "ended_at":end.to_rfc3339(),
            "paid_orders":self.paid, "refund_operations":self.refunds,
            "gross_received_usd":decimal(self.gross), "refunded_usd":decimal(self.returned),
            "net_received_usd":decimal(self.gross-self.returned),
            "platform_fees_collected_usd":decimal(self.fees), "definition":BOUNDARY})
    }
}

impl PostgresStore {
    /// No private customer, order, wallet, processor or event identifiers leave this method.
    /// Read-only and independent of intake/execution gates, so obligations remain reportable.
    pub async fn managed_card_payment_metrics(
        &self,
        account: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Value> {
        if account.is_empty() || start >= end {
            return Err(ManagedOrderStoreError::Integrity);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        // A receipt is unique per order/PaymentIntent. Repeated webhook deliveries cannot add sales.
        // First paid event time, not checkout creation or latest redelivery, assigns the period.
        let rows = sqlx::query(r#"
            SELECT q.*, p.amount_minor AS paid_amount, p.currency AS paid_currency,
                   p.quote_digest AS paid_digest, p.order_id, p.payment_intent_id, o.accepted_at, paid.at AS paid_at
            FROM managed_stripe_paid_receipts p
            JOIN managed_service_orders o ON o.order_id=p.order_id
            JOIN managed_service_quotes q ON q.quote_id=o.quote_id
            LEFT JOIN LATERAL (SELECT min(event_created_at) AS at FROM managed_stripe_event_inbox e
                WHERE e.order_id=p.order_id AND e.stripe_account_id=p.stripe_account_id
                  AND e.livemode AND e.disposition='processed' AND e.outcome='paid') paid ON true
            WHERE p.livemode AND p.stripe_account_id=$1 AND (paid.at IS NULL OR to_timestamp(paid.at)<$2)
            ORDER BY p.order_id LIMIT 5001
        "#).bind(account).bind(end).fetch_all(&mut *tx).await?;
        if rows.len() > LIMIT {
            return Err(ManagedOrderStoreError::Integrity);
        }
        let mut quotes = std::collections::HashMap::new();
        let mut totals = Totals::default();
        for row in rows {
            let quote = quote_from_row(&row)?;
            let input = quote.input();
            let expected_currency = match input.currency {
                FiatCurrency::Usd => "usd",
                FiatCurrency::Mxn => "mxn",
            };
            if !input.livemode
                || input.stripe_account_id != account
                || row.try_get::<String, _>("paid_currency")? != expected_currency
                || row.try_get::<String, _>("paid_digest")? != quote.digest()?
                || row.try_get::<i64, _>("paid_amount")?
                    != i64::try_from(quote.total_minor())
                        .map_err(|_| ManagedOrderStoreError::Integrity)?
            {
                return Err(ManagedOrderStoreError::Integrity);
            }
            let at: i64 = row.try_get("paid_at")?;
            // Compare timestamps, preserving microsecond-exclusive boundaries from the parent API.
            let at = DateTime::from_timestamp(at, 0).ok_or(ManagedOrderStoreError::Integrity)?;
            if at >= start {
                totals.paid += 1;
                totals.gross = totals
                    .gross
                    .checked_add(usd_micro(
                        quote.total_minor(),
                        input.currency,
                        &input.price_basis,
                    )?)
                    .ok_or(ManagedOrderStoreError::Integrity)?;
                let fee = quote
                    .lines()
                    .iter()
                    .find(|l| l.code == "platform_fee")
                    .ok_or(ManagedOrderStoreError::Integrity)?;
                totals.fees = totals
                    .fees
                    .checked_add(usd_micro(
                        fee.amount_minor,
                        input.currency,
                        &input.price_basis,
                    )?)
                    .ok_or(ManagedOrderStoreError::Integrity)?;
            }
            quotes.insert(
                row.try_get::<Uuid, _>("order_id")?,
                (
                    quote,
                    row.try_get::<String, _>("payment_intent_id")?,
                    row.try_get::<i64, _>("accepted_at")?,
                ),
            );
        }
        // Tables and kinds are fixed source constants, never caller-supplied SQL.
        for (table, kind) in [
            ("managed_refund", "full"),
            ("managed_deposit_refund", "review_deposit"),
            ("managed_recovered_refund", "recovered_bounty"),
        ] {
            let sql = format!(
                r#"
                SELECT f.order_id, to_jsonb(f) AS operation, ev.evidence, ev.outcome, ev.succeeded_at
                FROM {table}_operations f
                JOIN LATERAL (
                    SELECT (array_agg(e.outcome ORDER BY CASE e.outcome WHEN 'failed' THEN 4 WHEN 'canceled' THEN 4 WHEN 'succeeded' THEN 3 ELSE 0 END DESC, e.event_created_at DESC))[1] AS outcome,
                        (array_agg(to_jsonb(e) ORDER BY CASE e.outcome WHEN 'failed' THEN 4 WHEN 'canceled' THEN 4 WHEN 'succeeded' THEN 3 ELSE 0 END DESC, e.event_created_at DESC))[1] AS evidence,
                        min(e.event_created_at) FILTER(WHERE e.outcome='succeeded') AS succeeded_at
                    FROM {table}_events e WHERE e.order_id=f.order_id AND e.operation_id=f.operation_id
                        AND e.stripe_account_id=f.stripe_account_id AND e.livemode=f.livemode
                        AND e.disposition='processed' AND to_timestamp(e.event_created_at)<$2
                ) ev ON ev.outcome='succeeded'
                WHERE f.livemode AND f.stripe_account_id=$1 AND f.refund_id IS NOT NULL
                  AND to_timestamp(ev.succeeded_at)>=$3 ORDER BY f.order_id LIMIT 5001
            "#
            );
            let refunds = sqlx::query(&sql)
                .bind(account)
                .bind(end)
                .bind(start)
                .fetch_all(&mut *tx)
                .await?;
            if refunds.len() > LIMIT {
                return Err(ManagedOrderStoreError::Integrity);
            }
            for row in refunds {
                let id: Uuid = row.try_get("order_id")?;
                let (quote, pi, accepted_at) =
                    quotes.get(&id).ok_or(ManagedOrderStoreError::Integrity)?;
                let kind_value = match kind {
                    "full" => managed_orders::stripe_refund::RefundKind::Full,
                    "review_deposit" => managed_orders::stripe_refund::RefundKind::ReviewDeposit,
                    _ => managed_orders::stripe_refund::RefundKind::RecoveredBounty,
                };
                let operation: Value = row.try_get("operation")?;
                let plan =
                    super::refunds::plan(&operation, quote, id, *accepted_at, pi, kind_value)?;
                let evidence = super::refunds::event(&row.try_get::<Value, _>("evidence")?)?;
                if !plan.matches_event(&evidence)
                    || operation["refund_id"] != evidence.refund_id
                    || operation["charge_id"] != evidence.charge_id
                {
                    return Err(ManagedOrderStoreError::Integrity);
                }
                let input = quote.input();
                totals.refunds += 1;
                totals.returned = totals
                    .returned
                    .checked_add(usd_micro(
                        plan.amount_minor(),
                        input.currency,
                        &input.price_basis,
                    )?)
                    .ok_or(ManagedOrderStoreError::Integrity)?;
                if kind == "full" {
                    let fee = quote
                        .lines()
                        .iter()
                        .find(|l| l.code == "platform_fee")
                        .ok_or(ManagedOrderStoreError::Integrity)?;
                    totals.fees = totals
                        .fees
                        .checked_sub(usd_micro(
                            fee.amount_minor,
                            input.currency,
                            &input.price_basis,
                        )?)
                        .ok_or(ManagedOrderStoreError::Integrity)?;
                }
            }
        }
        tx.rollback().await?;
        Ok(totals.response(start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversions_are_integer_and_usd_does_not_use_mxn_quote_basis() {
        let basis = PriceBasis {
            reference: "fixture".into(),
            minor_units_per_usdc_numerator: 1890,
            minor_units_per_usdc_denominator: 1,
            valid_until: 0,
        };
        assert_eq!(usd_micro(1000, FiatCurrency::Mxn, &basis).unwrap(), 529100);
        assert_eq!(usd_micro(35, FiatCurrency::Mxn, &basis).unwrap(), 18518);
        assert_eq!(
            usd_micro(1000, FiatCurrency::Usd, &basis).unwrap(),
            10_000_000
        );
        assert_eq!(decimal(-18518), "-0.018518");
        let bad = PriceBasis {
            minor_units_per_usdc_numerator: 0,
            ..basis
        };
        assert!(usd_micro(35, FiatCurrency::Mxn, &bad).is_err());
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::managed_service_orders::{
        refunds::tests::{object, signed},
        stripe_tests,
        tests::{accept, Fixture},
    };
    use managed_orders::stripe_checkout::CheckoutPolicy;

    async fn live_paid(
        f: &Fixture,
        currency: FiatCurrency,
    ) -> (
        ManagedOrder,
        managed_orders::stripe_checkout::ManagedCheckoutPlan,
    ) {
        let mut input = f.quote().await.input().clone();
        input.livemode = true;
        input.expires_at = f.now().await + 7200;
        input.price_basis.valid_until = input.expires_at;
        input.bounty.network = "base-mainnet".into();
        input.currency = currency;
        if currency == FiatCurrency::Usd {
            input.price_basis.minor_units_per_usdc_numerator = 100;
        }
        let quote = ManagedQuote::new(input).unwrap();
        f.store.put_managed_service_quote(&quote).await.unwrap();
        let order = accept(&f.store, &quote, &Uuid::new_v4().to_string())
            .await
            .unwrap();
        let policy = CheckoutPolicy {
            stripe_account_id: quote.input().stripe_account_id.clone(),
            livemode: true,
            currency,
            minimum_minor: if currency == FiatCurrency::Usd {
                50
            } else {
                1000
            },
            maximum_minor: 1_000_000,
            payment_method_configuration: None,
            adaptive_pricing: false,
        };
        let p = f
            .store
            .prepare_managed_checkout("fixture-buyer", order.order_id, policy)
            .await
            .unwrap();
        let mut response = stripe_tests::session(
            &p,
            &format!("cs_live_{}", order.order_id.simple()),
            &format!("pi_{}", order.order_id.simple()),
        );
        response["currency"] = json!(if currency == FiatCurrency::Usd {
            "usd"
        } else {
            "mxn"
        });
        f.store
            .bind_managed_checkout(order.order_id, &p.session_binding(&response).unwrap())
            .await
            .unwrap();
        f.store
            .reconcile_managed_checkout_event(&stripe_tests::event(
                &p,
                &format!("evt_{}", order.order_id.simple()),
                "checkout.session.completed",
                response,
                0,
            ))
            .await
            .unwrap();
        (order, p)
    }
    async fn metrics(f: &Fixture, start: i64, end: i64) -> Value {
        f.store
            .managed_card_payment_metrics(
                "acct_fixture_only",
                DateTime::from_timestamp(start, 0).unwrap(),
                DateTime::from_timestamp(end, 0).unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn mixed_live_receipts_signed_refunds_windows_replay_and_privacy() {
        let f = Fixture::new().await;
        let (order, p) = live_paid(&f, FiatCurrency::Mxn).await;
        let (_usd, _) = live_paid(&f, FiatCurrency::Usd).await;
        let _sandbox = stripe_tests::paid(&f).await;
        let start = p.created_at() - 60;
        let end = p.created_at() + 60;
        let before = metrics(&f, start, end).await;
        assert_eq!(before["paid_orders"], 2);
        assert_eq!(before["gross_received_usd"], "225.000000");
        assert_eq!(before["platform_fees_collected_usd"], "15.000000");
        assert_eq!(before["refund_operations"], 0);
        assert_eq!(metrics(&f, start, p.created_at()).await["paid_orders"], 0);
        let other = f
            .store
            .managed_card_payment_metrics(
                "acct_other",
                DateTime::from_timestamp(start, 0).unwrap(),
                DateTime::from_timestamp(end, 0).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(other["paid_orders"], 0);
        let mut session = stripe_tests::session(
            &p,
            &format!("cs_live_{}", order.order_id.simple()),
            &format!("pi_{}", order.order_id.simple()),
        );
        session["currency"] = json!("mxn");
        let duplicate = stripe_tests::event(
            &p,
            "evt_later_paid",
            "checkout.session.completed",
            session,
            5,
        );
        f.store
            .reconcile_managed_checkout_event(&duplicate)
            .await
            .unwrap();
        assert_eq!(metrics(&f, start, end).await, before);
        assert_eq!(metrics(&f, f.now().await + 1, end).await["paid_orders"], 0);
        f.store
            .request_managed_refund("fixture-buyer", order.order_id)
            .await
            .unwrap();
        let plan = f
            .store
            .get_managed_refund_plan("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .unwrap();
        let lease = f
            .store
            .claim_managed_refund(order.order_id)
            .await
            .unwrap()
            .unwrap();
        f.store
            .complete_managed_refund(&lease, &plan.binding(&object(&plan, "succeeded")).unwrap())
            .await
            .unwrap();
        assert_eq!(
            metrics(&f, start, end).await["refund_operations"],
            0,
            "API success is not refund evidence"
        );
        let success = signed(&plan, "evt_metrics_refund", object(&plan, "succeeded"), 10);
        f.store
            .reconcile_managed_refund_event(&success)
            .await
            .unwrap();
        f.store
            .reconcile_managed_refund_event(&success)
            .await
            .unwrap();
        f.store
            .reconcile_managed_refund_event(&signed(
                &plan,
                "evt_duplicate_success",
                object(&plan, "succeeded"),
                15,
            ))
            .await
            .unwrap();
        let after = metrics(&f, start, end).await;
        assert_eq!(after["refund_operations"], 1);
        assert_eq!(after["refunded_usd"], "112.500000");
        assert_eq!(after["net_received_usd"], "112.500000");
        assert_eq!(after["platform_fees_collected_usd"], "7.500000");
        let only_refund = metrics(&f, plan.created_at() + 1, end).await;
        assert_eq!(only_refund["gross_received_usd"], "0.000000");
        assert_eq!(only_refund["net_received_usd"], "-112.500000");
        assert_eq!(only_refund["platform_fees_collected_usd"], "-7.500000");
        assert_eq!(
            metrics(&f, plan.created_at() + 11, end).await["refund_operations"],
            0,
            "redelivery must not move the refund window"
        );
        assert_eq!(
            metrics(&f, start, plan.created_at() + 10).await["refund_operations"],
            0
        );
        let text = after.to_string();
        for private in [
            order.order_id.to_string(),
            "fixture-buyer".into(),
            "acct_fixture_only".into(),
            "cs_live_".into(),
            "evt_".into(),
            "re_".into(),
        ] {
            assert!(!text.contains(&private));
        }
        f.store
            .reconcile_managed_refund_event(&signed(
                &plan,
                "evt_late_failure",
                object(&plan, "failed"),
                20,
            ))
            .await
            .unwrap();
        assert_eq!(metrics(&f, start, end).await["refund_operations"], 0);
        assert_eq!(
            metrics(&f, start, plan.created_at() + 20).await["refund_operations"],
            1
        );
        sqlx::query(
            "UPDATE managed_stripe_paid_receipts SET amount_minor=amount_minor+1 WHERE order_id=$1",
        )
        .bind(order.order_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
        assert!(f
            .store
            .managed_card_payment_metrics(
                "acct_fixture_only",
                DateTime::from_timestamp(start, 0).unwrap(),
                DateTime::from_timestamp(end, 0).unwrap()
            )
            .await
            .is_err());
        f.cleanup().await;
    }
}
