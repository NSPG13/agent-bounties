use super::*;
use crate::managed_service_orders::{funding::tests::reserved, tests::Fixture, ManagedOrder};
use managed_orders::{review::ReviewAppeal, OrderError};
use std::sync::atomic::{AtomicUsize, Ordering};

const TX: &str = "0x00000000000000000000000000000000000000000000000000000000000000aa";
const VERIFIER: &str = "0xa0ee7a142d267c1f36714e4a8f75612f20a79720";

pub(in crate::managed_service_orders) async fn funded(f: &Fixture) -> ManagedOrder {
    let (order, _) = reserved(f).await;
    f.store
        .record_managed_funding_funded(order.order_id, TX, 42)
        .await
        .unwrap();
    order
}
pub(in crate::managed_service_orders) fn verdict(
    order: &ManagedOrder,
    passed: bool,
) -> CustomerVerdict {
    CustomerVerdict {
        bounty_contract: order.quote.input().bounty.predicted_bounty.clone(),
        round: 1,
        submission_hash: format!("0x{}", "11".repeat(32)),
        evidence_hash: format!("0x{}", "22".repeat(32)),
        passed,
        reason: if passed {
            String::new()
        } else {
            "The delivered report omits the benchmark.".into()
        },
    }
}
pub(in crate::managed_service_orders) const DEADLINE: u64 = 1_900_000_000;
/// Appeal periods in these tests end here, long before `DEADLINE`.
pub(in crate::managed_service_orders) const ENDS: u64 = 1_800_000_000;

fn ends(verdict: &CustomerVerdict) -> Option<u64> {
    (!verdict.passed).then_some(ENDS)
}
pub(in crate::managed_service_orders) fn signer(calls: &AtomicUsize) -> impl ReviewSigner + '_ {
    move |_, _, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(ReviewSignature {
            verifier: VERIFIER.into(),
            signature: format!("0x{}", "5".repeat(130)),
        })
    }
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn one_signed_verdict_per_round_and_retries_return_it() {
    let f = Fixture::new().await;
    let order = funded(&f).await;
    let calls = AtomicUsize::new(0);
    let pass = verdict(&order, true);
    let recorded = f
        .store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &pass,
            1_900_000_000,
            ends(&pass),
            signer(&calls),
        )
        .await
        .unwrap();
    assert_eq!(recorded.verdict, pass);
    assert_eq!(recorded.response_hash, pass.response_hash().unwrap());
    let retry = f
        .store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &pass,
            1_900_000_999,
            ends(&pass),
            signer(&calls),
        )
        .await
        .unwrap();
    assert_eq!((retry, calls.load(Ordering::SeqCst)), (recorded.clone(), 1));
    assert!(matches!(
        f.store
            .record_managed_customer_review(
                "fixture-buyer",
                order.order_id,
                &verdict(&order, false),
                1_900_000_000,
                ends(&verdict(&order, false)),
                signer(&calls)
            )
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    assert!(matches!(
        f.store
            .record_managed_customer_review(
                "other-buyer",
                order.order_id,
                &pass,
                1_900_000_000,
                ends(&pass),
                signer(&calls)
            )
            .await,
        Err(ManagedOrderStoreError::NotFound)
    ));
    assert!(f
        .store
        .list_managed_customer_reviews("other-buyer", order.order_id)
        .await
        .unwrap()
        .is_empty());
    let mut next = verdict(&order, false);
    next.round = 2;
    f.store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &next,
            1_900_000_000,
            ends(&next),
            signer(&calls),
        )
        .await
        .unwrap();
    let listed = f
        .store
        .list_managed_customer_reviews("fixture-buyer", order.order_id)
        .await
        .unwrap();
    assert_eq!(
        listed.iter().map(|r| r.verdict.round).collect::<Vec<_>>(),
        [2, 1]
    );
    // The round 2 rejection waits unsigned for an appeal.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(listed[0].signed.is_none() && listed[1].signed.is_some());
    sqlx::query("UPDATE managed_customer_reviews SET verdict=jsonb_set(verdict,'{reason}','\"edited\"') WHERE round=2")
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .list_managed_customer_reviews("fixture-buyer", order.order_id)
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn unfunded_orders_and_other_bounties_are_never_signed() {
    let f = Fixture::new().await;
    let calls = AtomicUsize::new(0);
    let (reserved_only, _) = reserved(&f).await;
    let order = funded(&f).await;
    let mut other_bounty = verdict(&order, true);
    other_bounty.bounty_contract = format!("0x{}", "ee".repeat(20));
    for (id, v) in [
        (reserved_only.order_id, verdict(&reserved_only, true)),
        (order.order_id, other_bounty),
    ] {
        assert!(matches!(
            f.store
                .record_managed_customer_review(
                    "fixture-buyer",
                    id,
                    &v,
                    1_900_000_000,
                    ends(&v),
                    signer(&calls)
                )
                .await,
            Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
        ));
    }
    let mut short = verdict(&order, false);
    short.reason = "No.".into();
    assert!(f
        .store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &short,
            1_900_000_000,
            ends(&short),
            signer(&calls)
        )
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_customer_reviews")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn concurrent_opposite_verdicts_record_exactly_one() {
    let f = Fixture::new().await;
    let order = funded(&f).await;
    let mut tasks = vec![];
    for i in 0..8 {
        let (store, v) = (f.store.clone(), verdict(&order, i % 2 == 0));
        let id = order.order_id;
        tasks.push(tokio::spawn(async move {
            let calls = AtomicUsize::new(0);
            store
                .record_managed_customer_review(
                    "fixture-buyer",
                    id,
                    &v,
                    1_900_000_000,
                    ends(&v),
                    signer(&calls),
                )
                .await
                .map(|review| review.verdict.passed)
        }));
    }
    let mut outcomes = vec![];
    for task in tasks {
        outcomes.push(task.await.unwrap());
    }
    let winners: Vec<bool> = outcomes
        .iter()
        .filter_map(|o| o.as_ref().ok().copied())
        .collect();
    assert_eq!(
        winners.len(),
        4,
        "every request with the recorded verdict sees it"
    );
    assert!(winners.windows(2).all(|w| w[0] == w[1]));
    assert!(outcomes
        .iter()
        .filter(|o| o.is_err())
        .all(|o| matches!(o, Err(ManagedOrderStoreError::Quote(OrderError::Conflict)))));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_customer_reviews")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}

pub(in crate::managed_service_orders) fn appeal_of(
    review: &ManagedCustomerReview,
    reason: &str,
) -> ReviewAppeal {
    ReviewAppeal {
        bounty_contract: review.verdict.bounty_contract.clone(),
        round: review.verdict.round,
        submission_hash: review.verdict.submission_hash.clone(),
        rejection_hash: review.response_hash.clone(),
        solver: format!("0x{}", "5e".repeat(20)),
        reason: reason.into(),
    }
}
const APPEAL: &str = "Section 3 of the report contains the benchmark.";
const SIG: &str = "0xabcdef";

async fn rejected(f: &Fixture, calls: &AtomicUsize) -> ManagedCustomerReview {
    let order = funded(f).await;
    f.store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &verdict(&order, false),
            DEADLINE,
            Some(ENDS),
            signer(calls),
        )
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn an_unappealed_rejection_is_signed_only_after_its_appeal_period() {
    let f = Fixture::new().await;
    let calls = AtomicUsize::new(0);
    let review = rejected(&f, &calls).await;
    assert_eq!(
        (
            review.signed.as_ref(),
            review.appeal_ends_at,
            review.decided_by
        ),
        (None, Some(ENDS), ReviewDecider::Customer)
    );
    assert_eq!(review.appeal_status(ENDS - 1), Some("window_open"));
    let id = review.order_id;
    assert!(matches!(
        f.store
            .finalize_managed_rejection(id, 1, ENDS - 1, signer(&calls))
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    // Once the period has closed the solver can no longer appeal.
    assert!(f
        .store
        .record_managed_review_appeal(id, &appeal_of(&review, APPEAL), SIG, ENDS)
        .await
        .is_err());
    let signed = f
        .store
        .finalize_managed_rejection(id, 1, ENDS, signer(&calls))
        .await
        .unwrap();
    let outcome = signed.signed.clone().unwrap();
    assert_eq!(
        (outcome.passed, outcome.response_hash.as_str()),
        (false, review.response_hash.as_str())
    );
    assert_eq!(signed.appeal_status(ENDS), Some("final"));
    let again = f
        .store
        .finalize_managed_rejection(id, 1, ENDS + 9, signer(&calls))
        .await
        .unwrap();
    assert_eq!((again, calls.load(Ordering::SeqCst)), (signed, 1));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn an_appeal_blocks_the_rejection_until_one_signed_ruling() {
    let f = Fixture::new().await;
    let calls = AtomicUsize::new(0);
    let review = rejected(&f, &calls).await;
    let id = review.order_id;
    let appeal = appeal_of(&review, APPEAL);
    let mut wrong = appeal.clone();
    wrong.rejection_hash = format!("0x{}", "77".repeat(32));
    assert!(f
        .store
        .record_managed_review_appeal(id, &wrong, SIG, ENDS - 1)
        .await
        .is_err());
    let appealed = f
        .store
        .record_managed_review_appeal(id, &appeal, SIG, ENDS - 1)
        .await
        .unwrap();
    assert_eq!(appealed.appeal_status(ENDS + 1), Some("appealed"));
    assert_eq!(
        f.store
            .record_managed_review_appeal(id, &appeal, SIG, ENDS - 1)
            .await
            .unwrap(),
        appealed
    );
    assert!(matches!(
        f.store
            .record_managed_review_appeal(
                id,
                &appeal_of(&review, "A different reason for the appeal."),
                SIG,
                ENDS - 1
            )
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    // An appealed rejection is never signed on its own.
    assert!(f
        .store
        .finalize_managed_rejection(id, 1, ENDS + 1, signer(&calls))
        .await
        .is_err());
    assert_eq!(
        f.store.list_open_managed_appeals(10).await.unwrap(),
        vec![("base-sepolia".to_owned(), appealed.clone())]
    );
    let reason = "Section 3 contains the requested benchmark.";
    let ruled = f
        .store
        .record_managed_review_ruling(id, 1, true, reason, "operator", signer(&calls))
        .await
        .unwrap();
    let ruling = ruled.ruling.clone().unwrap();
    let outcome = ruled.signed.clone().unwrap();
    assert_eq!(
        (outcome.passed, &outcome.response_hash),
        (true, &ruling.response_hash)
    );
    assert_eq!(
        ruling.ruling.appeal_hash,
        appealed.appeal.unwrap().appeal_hash
    );
    assert!(ruled.reversed());
    assert_eq!(ruled.appeal_status(0), Some("reversed"));
    assert_eq!(
        f.store
            .record_managed_review_ruling(id, 1, true, reason, "operator", signer(&calls))
            .await
            .unwrap(),
        ruled
    );
    assert!(f
        .store
        .record_managed_review_ruling(id, 1, false, reason, "operator", signer(&calls))
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(f
        .store
        .list_open_managed_appeals(10)
        .await
        .unwrap()
        .is_empty());
    let (found, reviews) = f
        .store
        .managed_reviews_for_bounty("base-sepolia", &review.verdict.bounty_contract)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((found, reviews), (id, vec![ruled]));
    // The row check refuses a ruling without its signature.
    let unsigned = sqlx::query(
        "UPDATE managed_customer_reviews SET signature=NULL, verifier=NULL, signed_passed=NULL,
                signed_response_hash=NULL, signed_at=NULL WHERE order_id=$1",
    )
    .bind(id)
    .execute(&f.store.pool)
    .await;
    assert!(unsigned.is_err());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn silence_records_a_signed_platform_acceptance_once() {
    let f = Fixture::new().await;
    let calls = AtomicUsize::new(0);
    let order = funded(&f).await;
    let mut auto = verdict(&order, true);
    auto.reason = managed_orders::review::AUTO_ACCEPT_REASON.into();
    assert!(f
        .store
        .record_managed_deadline_acceptance(
            order.order_id,
            &verdict(&order, false),
            DEADLINE,
            signer(&calls)
        )
        .await
        .is_err());
    let accepted = f
        .store
        .record_managed_deadline_acceptance(order.order_id, &auto, DEADLINE, signer(&calls))
        .await
        .unwrap();
    assert_eq!(accepted.decided_by, ReviewDecider::Deadline);
    assert!(accepted.signed.as_ref().is_some_and(|s| s.passed));
    // The customer can no longer record a different verdict for the round.
    assert!(f
        .store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &verdict(&order, false),
            DEADLINE,
            Some(ENDS),
            signer(&calls)
        )
        .await
        .is_err());
    assert_eq!(
        f.store
            .record_managed_deadline_acceptance(order.order_id, &auto, DEADLINE, signer(&calls))
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.store
            .claim_managed_funded_bounties("base-sepolia", 10, 0)
            .await
            .unwrap(),
        vec![(order.order_id, auto.bounty_contract.clone())]
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn review_scan_rotates_after_crash_restart_and_concurrent_claims() {
    use std::collections::HashSet;
    let f = Fixture::new().await;
    let mut all = HashSet::new();
    for _ in 0..501 {
        all.insert(funded(&f).await.order_id);
    }
    // Two API instances reserve disjoint work, even if neither processes any of it.
    let (a, b) = tokio::join!(
        f.store
            .claim_managed_funded_bounties("base-sepolia", 250, 60),
        f.store
            .claim_managed_funded_bounties("base-sepolia", 250, 60)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!((a.len(), b.len()), (250, 250));
    let mut visited: HashSet<_> = a.iter().chain(&b).map(|(id, _)| *id).collect();
    assert_eq!(visited.len(), 500);
    let restarted = f.reconnect().await;
    restarted.migrate().await.unwrap();
    let tail = restarted
        .claim_managed_funded_bounties("base-sepolia", 500, 60)
        .await
        .unwrap();
    assert_eq!(tail.len(), 1);
    visited.extend(tail.iter().map(|(id, _)| *id));
    assert_eq!(visited, all);
    assert!(restarted
        .claim_managed_funded_bounties("base-sepolia", 500, 60)
        .await
        .unwrap()
        .is_empty());
    // Later inserts are visited even when every older order remains active.
    let later = funded(&f).await;
    assert_eq!(
        restarted
            .claim_managed_funded_bounties("base-sepolia", 500, 60)
            .await
            .unwrap()[0]
            .0,
        later.order_id
    );
    // A crash does not complete an obligation: all become eligible again after cooldown.
    sqlx::query("UPDATE managed_bounty_funding_operations SET last_review_attempt_at=last_review_attempt_at-interval '61 seconds'")
        .execute(&f.store.pool).await.unwrap();
    let again = restarted
        .claim_managed_funded_bounties("base-sepolia", 500, 60)
        .await
        .unwrap();
    assert_eq!(again.len(), 500);
    assert_eq!(
        again.iter().map(|(id, _)| *id).collect::<HashSet<_>>(),
        a.iter().chain(&b).map(|(id, _)| *id).collect()
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM managed_bounty_funding_operations WHERE outcome='funded'",
    )
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert_eq!(count, 502);
    restarted.pool.close().await;
    f.cleanup().await;
}
