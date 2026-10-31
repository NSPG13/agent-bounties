//! Opt-in loopback Anvil + disposable Postgres rehearsal of a card bounty's review life through
//! the real API, worker, finalizer, relayer and indexer: a rejection that becomes final after
//! its unused appeal period, a rejection the solver appeals and the arbiter reverses, and a
//! second bounty accepted because its customer stayed silent. Review time is advanced by
//! passing later clocks to the finalizer; on-chain time stays inside the verification window.
//! No hosted writes. Run `scripts/check-managed-review.sh`.
use super::*;
use crate::{managed_bounty_appeals::finalize_reviews_once, SharedState, X402HostedRelayerConfig};
use alloy::{signers::local::PrivateKeySigner, sol_types::SolValue};
use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request},
};
use chain_base::{sha256_canonical_json, sha256_utf8, BaseTransactionRelayer};
use managed_orders::review::{ReviewAppeal, APPEAL_SECONDS, DECISION_SECONDS};
use std::sync::Arc;
use tower::ServiceExt;
use worker::managed_funding::{
    poll_managed_funding_once,
    rehearsal_support::{
        address, balance, call_data, deploy, mint, pay, rpc, send, Chain, SOLVER_REWARD, TARGET,
        VERIFIER_REWARD,
    },
};

// Advance only scheduling metadata in this isolated schema. Review schedules still use the
// explicit business clock below. This avoids sleeping a real minute between rehearsal passes.
async fn finalize_after_interval(
    state: &SharedState,
    pool: &sqlx::PgPool,
    now: u64,
) -> Result<Vec<crate::managed_bounty_appeals::Finalized>, StatusCode> {
    sqlx::query("UPDATE managed_bounty_funding_operations SET last_review_attempt_at=last_review_attempt_at-interval '61 seconds'")
        .execute(pool).await.unwrap();
    finalize_reviews_once(state, now).await
}

/// Anvil account 9, the test-only pinned customer-review signer.
const REVIEW_KEY: &str = "0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6";
const OPERATOR_TOKEN: &str = "rehearsal-operator-token";

async fn call(
    state: &SharedState,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    *builder.headers_mut().unwrap() = headers.clone();
    let response = managed_payments::router()
        .with_state(state.clone())
        .oneshot(
            builder
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let code = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    (code, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The solver posts its bond, claims, and submits one artifact; returns the round's hashes.
async fn claim_and_submit(
    chain: &Chain,
    solver: &str,
    bounty: &str,
    artifact: &str,
) -> (String, String, Value) {
    let url = &chain.url;
    mint(url, &chain.owner, &chain.token, solver, VERIFIER_REWARD).await;
    send(
        url,
        solver,
        Some(&chain.token),
        call_data(
            "approve(address,uint256)",
            &(
                address(bounty),
                alloy::primitives::U256::from(VERIFIER_REWARD),
            )
                .abi_encode_params(),
        ),
    )
    .await;
    send(url, solver, Some(bounty), call_data("claim()", &[])).await;
    let evidence = json!({"artifact_url": artifact,
        "artifact_sha256": format!("sha256:{}", "ab".repeat(32))});
    let submission_hash = sha256_utf8(artifact);
    let evidence_hash = sha256_canonical_json(&evidence).unwrap();
    let words = (
        submission_hash.parse::<alloy::primitives::B256>().unwrap(),
        evidence_hash.parse::<alloy::primitives::B256>().unwrap(),
    );
    send(
        url,
        solver,
        Some(bounty),
        call_data("submit(bytes32,bytes32)", &words.abi_encode_params()),
    )
    .await;
    (submission_hash, evidence_hash, evidence)
}

async fn publish_evidence(
    state: &SharedState,
    store: &db::PostgresStore,
    bounty: &str,
    solver: &str,
    round: u64,
    artifact: &str,
    evidence: Value,
) {
    let item = crate::indexed_autonomous_bounty(state, "base-sepolia", bounty)
        .await
        .unwrap();
    let record = chain_base::build_autonomous_submission_evidence_record(
        "base-sepolia",
        &item,
        bounty,
        &item.bounty_id,
        round,
        solver,
        artifact,
        evidence,
        chrono::Utc::now(),
    )
    .unwrap();
    store
        .upsert_autonomous_submission_evidence(&record)
        .await
        .unwrap();
}

async fn status_of(state: &SharedState, bounty: &str) -> AutonomousBountyFeedItem {
    crate::indexed_autonomous_bounty(state, "base-sepolia", bounty)
        .await
        .unwrap()
}

fn has(item: &AutonomousBountyFeedItem, kind: AutonomousBountyEventKind) -> usize {
    item.events.iter().filter(|e| e.kind == kind).count()
}

/// A solver with a real key, so it can sign its appeal; Anvil sends its transactions.
async fn solver(url: &str) -> (PrivateKeySigner, String) {
    let key = PrivateKeySigner::random();
    let address = format!("{:#x}", key.address());
    rpc(
        url,
        "anvil_setBalance",
        json!([address, "0x8ac7230489e80000"]),
    )
    .await;
    rpc(url, "anvil_impersonateAccount", json!([address])).await;
    (key, address)
}

async fn fund(
    state: &SharedState,
    chain: &Chain,
    store: &db::PostgresStore,
    buyer: &str,
    nonce: &str,
) -> (Uuid, String) {
    let (quote, bounty) = chain.bounty_quote(store, buyer, nonce).await;
    let order = pay(store, &quote).await;
    let funding = chain.funding_config();
    assert_eq!(
        poll_managed_funding_once(store, &funding)
            .await
            .unwrap()
            .action,
        "signed"
    );
    chain.index(store).await;
    assert_eq!(
        poll_managed_funding_once(store, &funding)
            .await
            .unwrap()
            .action,
        "funded"
    );
    assert_eq!(status_of(state, &bounty).await.status, "claimable");
    (order, bounty)
}

#[tokio::test]
#[ignore = "run scripts/check-managed-review.sh with a disposable loopback database"]
async fn rejections_wait_for_appeal_rulings_and_silence_settle_only_through_the_contract() {
    let url = std::env::var("AGENT_BOUNTIES_TEST_RPC_URL").unwrap();
    let database = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&database);
    let schema = format!("managed_review_fairness_{}", Uuid::new_v4().simple());
    let admin = sqlx::PgPool::connect(&database).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _| {
            let statement = format!("SET search_path TO {schema}");
            Box::pin(async move {
                sqlx::query(&statement).execute(connection).await?;
                Ok(())
            })
        })
        .connect(&database)
        .await
        .unwrap();
    let store = db::PostgresStore::from_pool(pool.clone());
    store.migrate().await.unwrap();
    let chain = deploy(&url, &store, 3 * TARGET).await;
    let (site, account, headers) =
        crate::site_auth::managed_payment_account_fixture(Some(store.clone()), "rehearsal-owner");
    let (_, _, other) =
        crate::site_auth::managed_payment_account_fixture(Some(store.clone()), "rehearsal-other");
    let buyer = format!("account:{account}");
    let mut operator = HeaderMap::new();
    operator.insert("x-operator-token", OPERATOR_TOKEN.parse().unwrap());

    // Sponsored relayer on the same loopback chain.
    let relayer =
        BaseTransactionRelayer::from_private_key(&format!("{:064x}", Uuid::new_v4().as_u128()))
            .unwrap()
            .with_gas_budget(store.gas_sponsor_budget());
    rpc(
        &url,
        "anvil_setBalance",
        json!([relayer.address(), "0x8ac7230489e80000"]),
    )
    .await;
    std::env::set_var("ENABLE_SPONSORED_BOUNTY_CREATION", "true");
    let mut state = crate::tests::test_state(app::BountyNetwork::default());
    let s = Arc::make_mut(&mut state);
    s.store = Some(store.clone());
    s.operator_api_token = Some(OPERATOR_TOKEN.into());
    s.security_auth = crate::security_auth::SecurityAuthState::for_managed_payment_test(site);
    s.base_rpc_urls.base_sepolia = Some(url.clone());
    s.x402_relayer = X402HostedRelayerConfig {
        enabled: true,
        relayer: Some(Arc::new(relayer)),
        max_daily_attempts: 100_000,
        max_daily_attempts_per_contributor: 100_000,
        confirmations: 1,
        wait_seconds: 2,
        rpc_timeout_seconds: 30,
        ..Default::default()
    };
    s.managed_payments.bounty = Some(serde_json::from_value(json!({
        "network": "base-sepolia", "funding_wallet": chain.wallet,
        "customer_review": {"verifier": chain_base::TEST_CUSTOMER_REVIEW_VERIFIER,
            "verifier_reward_micro_usdc": VERIFIER_REWARD, "claim_window_seconds": 604_800,
            "verification_window_seconds": 604_800, "public_disclosure": "Customer review."},
        "max_funding_window_seconds": 2_592_000, "quote_lifetime_seconds": 3_600,
        "minimum_open_seconds": 86_400, "terms_version": "rehearsal", "remedies": "Rehearsal",
        "pricing": {"conversion_buffer_bps": 100, "service_tax_bps": null, "processing_rate_bps": 410,
            "processing_fixed_minor": 300, "processing_fee_tax_bps": 1600},
        "rate_source": {"fixed": {"price": "18.70"}}
    })).unwrap());
    s.managed_payments.review_signer = Some(REVIEW_KEY.parse().unwrap());

    let (order, bounty) = fund(&state, &chain, &store, &buyer, "7c").await;
    let review_path = format!("/v1/managed-service/orders/{order}/review");
    let public_path = format!("/v1/managed-service/bounties/base-sepolia/{bounty}/review");
    let appeal_path = format!("/v1/managed-service/bounties/base-sepolia/{bounty}/appeal");
    let (code, view) = call(&state, "GET", &review_path, &headers, Value::Null).await;
    assert_eq!(code, StatusCode::OK, "{view}");
    assert!(view["pending_submission"].is_null());
    assert_eq!(view["deposit"]["status"], "held");
    assert_eq!(
        call(&state, "GET", &review_path, &other, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // Round 1: the customer cannot judge unpublished evidence, then rejects. The rejection
    // waits unsigned for the solver; only after the unused appeal period does the finalizer
    // sign it, and the solver's bond goes to the verifier while the bounty stays funded.
    let (key, solver_address) = solver(&url).await;
    let first = "https://example.com/round-1.txt";
    let (submission, evidence_hash, evidence) =
        claim_and_submit(&chain, &solver_address, &bounty, first).await;
    chain.index(&store).await;
    let reject = json!({"round": 1, "submission_hash": submission, "evidence_hash": evidence_hash,
        "passed": false, "reason": "The artifact does not contain the requested report."});
    assert_eq!(
        call(&state, "POST", &review_path, &headers, reject.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    publish_evidence(&state, &store, &bounty, &solver_address, 1, first, evidence).await;
    let (_, view) = call(&state, "GET", &review_path, &headers, Value::Null).await;
    let decide_by = view["pending_submission"]["decide_by"].as_u64().unwrap();
    let start = now();
    assert!((start + DECISION_SECONDS - 5..=start + DECISION_SECONDS).contains(&decide_by));
    let pending = &view["pending_submission"];
    assert_eq!(pending["auto_accept"], true);
    let sign_by = pending["sign_by"].as_u64().unwrap();
    assert!(decide_by < sign_by && sign_by < pending["verification_expires_at"].as_u64().unwrap());
    let mut short = reject.clone();
    short["reason"] = "Bad.".into();
    assert_eq!(
        call(&state, "POST", &review_path, &headers, short).await.0,
        StatusCode::BAD_REQUEST
    );
    let (code, rejected) = call(&state, "POST", &review_path, &headers, reject.clone()).await;
    assert_eq!(code, StatusCode::OK, "{rejected}");
    assert!(rejected["signature"].is_null() && rejected["attestation"].is_null());
    assert_eq!(rejected["appeal"]["status"], "window_open");
    assert_eq!(rejected["relay"]["status"], "awaiting_appeal_period");
    let window_ends = rejected["appeal"]["window_ends_at"].as_u64().unwrap();
    assert!(window_ends >= start + APPEAL_SECONDS);
    let (_, public) = call(&state, "GET", &public_path, &HeaderMap::new(), Value::Null).await;
    assert_eq!(
        public["reviews"][0]["appeal_typed_data"]["primaryType"],
        "ReviewAppeal"
    );
    assert!(public.to_string().find(&account).is_none());
    // Nothing is signed while the solver may still appeal.
    assert!(finalize_after_interval(&state, &pool, window_ends - 1)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(status_of(&state, &bounty).await.status, "submitted");
    let verifier_before = balance(
        &url,
        &chain.token,
        chain_base::TEST_CUSTOMER_REVIEW_VERIFIER,
    )
    .await;
    let done = finalize_after_interval(&state, &pool, window_ends)
        .await
        .unwrap();
    assert_eq!(done[0].action, "rejection_signed", "{done:?}");
    chain.index(&store).await;
    let item = status_of(&state, &bounty).await;
    assert_eq!(has(&item, AutonomousBountyEventKind::SubmissionRejected), 1);
    assert_ne!(item.status, "submitted");
    assert_eq!(
        balance(
            &url,
            &chain.token,
            chain_base::TEST_CUSTOMER_REVIEW_VERIFIER
        )
        .await,
        verifier_before + u128::from(VERIFIER_REWARD)
    );
    assert_eq!(balance(&url, &chain.token, &solver_address).await, 0);
    let (_, view) = call(&state, "GET", &review_path, &headers, Value::Null).await;
    assert_eq!(view["reviews"][0]["appeal"]["status"], "final");

    // Round 2: rejected again; the solver appeals with its wallet signature, which blocks the
    // rejection. The arbiter reverses it: the signed ruling pays the solver its reward and bond,
    // and the customer's review deposit is forfeited to the solver.
    let second = "https://example.com/round-2.txt";
    let (submission, evidence_hash, evidence) =
        claim_and_submit(&chain, &solver_address, &bounty, second).await;
    chain.index(&store).await;
    publish_evidence(
        &state,
        &store,
        &bounty,
        &solver_address,
        2,
        second,
        evidence,
    )
    .await;
    let reject = json!({"round": 2, "submission_hash": submission, "evidence_hash": evidence_hash,
        "passed": false, "reason": "The second artifact still omits the requested report."});
    let (code, rejected) = call(&state, "POST", &review_path, &headers, reject).await;
    assert_eq!(code, StatusCode::OK, "{rejected}");
    let reason = "The report is in section 2 of the delivered artifact.";
    let appeal = ReviewAppeal {
        bounty_contract: bounty.clone(),
        round: 2,
        submission_hash: submission.clone(),
        rejection_hash: rejected["response_hash"].as_str().unwrap().into(),
        solver: solver_address.clone(),
        reason: reason.into(),
    };
    let digest = crate::managed_bounty_appeals::appeal_digest("base-sepolia", &appeal).unwrap();
    let sign = |signer: &PrivateKeySigner| {
        format!(
            "0x{}",
            hex::encode(signer.sign_hash_sync(&digest).unwrap().as_bytes())
        )
    };
    let body = |signature: String| json!({"round": 2, "solver": solver_address, "reason": reason, "signature": signature});
    let (code, _) = call(
        &state,
        "POST",
        &appeal_path,
        &HeaderMap::new(),
        body(sign(&PrivateKeySigner::random())),
    )
    .await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
    let (code, appealed) = call(
        &state,
        "POST",
        &appeal_path,
        &HeaderMap::new(),
        body(sign(&key)),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{appealed}");
    assert_eq!(appealed["appeal"]["status"], "appealed");
    // An appealed rejection is never signed by the schedule.
    assert!(finalize_after_interval(&state, &pool, now() + 4 * 86_400)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        call(
            &state,
            "GET",
            "/v1/managed-service/appeals",
            &headers,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (code, queue) = call(
        &state,
        "GET",
        "/v1/managed-service/appeals",
        &operator,
        Value::Null,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{queue}");
    assert_eq!(queue["appeals"][0]["appeal"]["reason"], reason);
    assert_eq!(
        queue["appeals"][0]["evidence"]["artifact_reference"],
        second
    );
    let ruling_path = format!("/v1/managed-service/appeals/base-sepolia/{bounty}/2/ruling");
    let ruling = json!({"reversed": true, "reason": "Section 2 of the artifact contains the requested report."});
    assert_eq!(
        call(&state, "POST", &ruling_path, &headers, ruling.clone())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // A signed reversal and its deposit obligation must survive an unavailable relayer.
    // Restore relaying only after a finalizer pass has also observed the pending reversal.
    Arc::make_mut(&mut state).x402_relayer.enabled = false;
    let (code, ruled) = call(&state, "POST", &ruling_path, &operator, ruling).await;
    assert_eq!(code, StatusCode::OK, "{ruled}");
    assert_eq!(ruled["appeal"]["status"], "reversed");
    assert_eq!(ruled["attestation"]["passed"], true);
    assert_eq!(
        ruled["attestation"]["response_hash"],
        ruled["ruling"]["response_hash"]
    );
    assert_eq!(ruled["relay"]["status"], "deferred", "{ruled}");
    assert_eq!(ruled["deposit_payout"]["status"], "owed");
    let retry = finalize_after_interval(&state, &pool, now()).await.unwrap();
    assert!(retry.iter().any(|done| done.bounty == bounty
        && done.round == Some(2)
        && done.action == "relay_retried"));
    chain.index(&store).await;
    assert_eq!(
        has(
            &status_of(&state, &bounty).await,
            AutonomousBountyEventKind::BountySettled
        ),
        0
    );
    Arc::make_mut(&mut state).x402_relayer.enabled = true;
    let retry = finalize_after_interval(&state, &pool, now()).await.unwrap();
    assert!(retry.iter().any(|done| done.bounty == bounty
        && done.round == Some(2)
        && done.action == "relay_retried"));
    chain.index(&store).await;
    let item = status_of(&state, &bounty).await;
    assert_eq!(has(&item, AutonomousBountyEventKind::BountySettled), 1);
    assert_eq!(
        balance(&url, &chain.token, &solver_address).await,
        u128::from(SOLVER_REWARD + VERIFIER_REWARD)
    );
    assert_eq!(balance(&url, &chain.token, &bounty).await, 0);
    let (_, view) = call(&state, "GET", &review_path, &headers, Value::Null).await;
    assert_eq!(view["bounty_status"], "paid");
    assert_eq!(view["deposit"]["status"], "forfeited_to_solver");

    // The reversal owes the customer's deposit to the solver. Only a confirmed USDC transfer of
    // the owed amount to that solver records it as paid, and one hash pays one obligation.
    let owed = managed_orders::review::review_deposit_micro_usdc(SOLVER_REWARD);
    assert_eq!(ruled["deposit_payout"]["status"], "owed");
    assert_eq!(ruled["deposit_payout"]["amount_micro_usdc"], owed);
    let (code, payouts) = call(
        &state,
        "GET",
        "/v1/managed-service/deposit-payouts",
        &operator,
        Value::Null,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{payouts}");
    assert_eq!(payouts["payouts"][0]["solver"], solver_address.as_str());
    assert_eq!(payouts["payouts"][0]["token"], chain.token.as_str());
    mint(&url, &chain.owner, &chain.token, &chain.owner, 2 * owed).await;
    let transfer = |amount: u64| {
        call_data(
            "transfer(address,uint256)",
            &(
                address(&solver_address),
                alloy::primitives::U256::from(amount),
            )
                .abi_encode_params(),
        )
    };
    let short = send(&url, &chain.owner, Some(&chain.token), transfer(owed - 1)).await;
    let paid = send(&url, &chain.owner, Some(&chain.token), transfer(owed)).await;
    rpc(&url, "anvil_mine", json!(["0x3"])).await;
    let payout_path = format!("/v1/managed-service/deposit-payouts/base-sepolia/{bounty}");
    let record = |receipt: &Value| json!({"tx_hash": receipt["transactionHash"]});
    assert_eq!(
        call(&state, "POST", &payout_path, &headers, record(&paid))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&state, "POST", &payout_path, &operator, record(&short))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (code, recorded) = call(&state, "POST", &payout_path, &operator, record(&paid)).await;
    assert_eq!(code, StatusCode::OK, "{recorded}");
    assert_eq!(recorded["status"], "paid");
    assert_eq!(recorded["tx_hash"], paid["transactionHash"]);
    let (_, public) = call(&state, "GET", &public_path, &HeaderMap::new(), Value::Null).await;
    assert_eq!(public["deposit_payout"]["status"], "paid");
    let rounds: Vec<_> = view["reviews"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["document"]["round"].clone())
        .collect();
    assert_eq!(rounds, [json!(2), json!(1)]);

    // A second bounty whose customer stays silent: nothing happens before the decide-by time,
    // then the platform's acceptance is signed and settles through the contract.
    let (order, bounty) = fund(&state, &chain, &store, &buyer, "8d").await;
    let (_, silent_address) = solver(&url).await;
    let third = "https://example.com/silent.txt";
    let (_, _, evidence) = claim_and_submit(&chain, &silent_address, &bounty, third).await;
    chain.index(&store).await;
    // Unpublished evidence is never accepted by silence.
    assert!(
        finalize_after_interval(&state, &pool, now() + DECISION_SECONDS + 60)
            .await
            .unwrap()
            .is_empty()
    );
    publish_evidence(&state, &store, &bounty, &silent_address, 1, third, evidence).await;
    let published = now();
    assert!(
        finalize_after_interval(&state, &pool, published + DECISION_SECONDS - 60)
            .await
            .unwrap()
            .is_empty()
    );
    // A permanently malformed earlier candidate must not prevent the later silent acceptance.
    let (poison_order, poison_bounty) = fund(&state, &chain, &store, &buyer, "8e").await;
    let poison_artifact = "https://example.com/poison.txt";
    let (_, _, poison_evidence) =
        claim_and_submit(&chain, &silent_address, &poison_bounty, poison_artifact).await;
    chain.index(&store).await;
    publish_evidence(
        &state,
        &store,
        &poison_bounty,
        &silent_address,
        1,
        poison_artifact,
        poison_evidence,
    )
    .await;
    sqlx::query("UPDATE managed_service_quotes SET quote=jsonb_set(quote,'{total_minor}','1') WHERE quote_id=(SELECT quote_id FROM managed_service_orders WHERE order_id=$1)")
        .bind(poison_order)
        .execute(&pool)
        .await
        .unwrap();
    // Both are due, with the corrupt candidate first irrespective of random UUID ordering.
    sqlx::query("UPDATE managed_bounty_funding_operations SET last_review_attempt_at=CASE WHEN order_id=$1 THEN '2000-01-01'::timestamptz ELSE '2001-01-01'::timestamptz END")
        .bind(poison_order).execute(&pool).await.unwrap();
    assert!(
        finalize_after_interval(&state, &pool, now() + DECISION_SECONDS + 5)
            .await
            .is_err()
    );
    let (_, reviews) = store
        .managed_reviews_for_bounty("base-sepolia", &bounty)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reviews.len(), 1);
    assert!(reviews[0].verdict.passed && reviews[0].signed.is_some());
    chain.index(&store).await;
    assert_eq!(
        has(
            &status_of(&state, &bounty).await,
            AutonomousBountyEventKind::BountySettled
        ),
        1
    );
    assert_eq!(
        balance(&url, &chain.token, &silent_address).await,
        u128::from(SOLVER_REWARD + VERIFIER_REWARD)
    );
    let (_, view) = call(
        &state,
        "GET",
        &format!("/v1/managed-service/orders/{order}/review"),
        &headers,
        Value::Null,
    )
    .await;
    assert_eq!(view["reviews"][0]["decided_by"], "deadline");
    // Paid without a reversal: the next pass reserves the deposit refund to the card. Stripe
    // is not configured here, so it stays pending until a signed refund event arrives.
    assert_eq!(view["deposit"]["status"], "held");
    let done = finalize_after_interval(&state, &pool, now()).await.unwrap();
    assert!(
        done.iter().any(|d| d.action == "deposit_refund_reserved"),
        "{done:?}"
    );
    let (_, view) = call(
        &state,
        "GET",
        &format!("/v1/managed-service/orders/{order}/review"),
        &headers,
        Value::Null,
    )
    .await;
    assert_eq!(view["deposit"]["status"], "refund_pending");
    assert!(finalize_after_interval(&state, &pool, now())
        .await
        .unwrap()
        .is_empty());
}
