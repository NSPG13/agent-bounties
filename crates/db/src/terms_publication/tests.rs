use super::*;
use crate::{NewTrialBounty, PgPool};
use chrono::Duration;
use serde_json::json;

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn record(name: &str) -> AutonomousBountyTermsRecord {
    let document: domain::AutonomousBountyTermsDocument = serde_json::from_value(json!({
        "schema_version":"agent-bounties/terms-v1",
        "contract_terms":{"creator_wallet":"0x1111111111111111111111111111111111111111","network":"base-mainnet","funding_deadline":1},
        "title":name,"goal":"Isolated persistence fixture, never a contract plan",
        "acceptance_criteria":["One immutable record"],"benchmark":{"engine":"test-only"},
        "evidence_schema":{},"verification_policy":{},"source_url":null,"discovery_source":null
    })).unwrap();
    let h = |v: &serde_json::Value| chain_base::keccak256_canonical_json(v).unwrap();
    AutonomousBountyTermsRecord {
        terms_hash: h(&serde_json::to_value(&document).unwrap()),
        policy_hash: h(&document.verification_policy),
        acceptance_criteria_hash: h(&json!(document.acceptance_criteria)),
        benchmark_hash: h(&document.benchmark),
        evidence_schema_hash: h(&document.evidence_schema),
        creator_wallet: "0x1111111111111111111111111111111111111111".into(),
        document,
        created_at: Utc::now() - Duration::days(2),
    }
}
fn identity<'a>(
    principal: &'a str,
    key: &'a str,
    payload: &'a str,
) -> TermsPublicationIdentity<'a> {
    TermsPublicationIdentity {
        principal,
        operation: TermsPublicationOperation::Terms,
        key_hash: key,
        payload_hash: payload,
    }
}
async fn stores() -> (PostgresStore, PostgresStore, PgPool, String) {
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
        .expect("disposable local PostgreSQL required");
    crate::test_support::assert_isolated_security_database(&url);
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("terms_publication_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let first = PostgresStore::from_pool(PgPool::connect_with(options.clone()).await.unwrap());
    first.migrate().await.unwrap();
    let second = PostgresStore::from_pool(PgPool::connect_with(options).await.unwrap());
    (first, second, admin, schema)
}
async fn cleanup(first: PostgresStore, second: PostgresStore, admin: PgPool, schema: String) {
    first.pool.close().await;
    second.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
async fn counts(store: &PostgresStore) -> (i64, i64, i64, i64) {
    let row=sqlx::query("SELECT (SELECT count(*) FROM autonomous_bounty_terms) AS terms,(SELECT count(*) FROM security_resource_owners WHERE resource_kind='autonomous_terms') AS owners,(SELECT count(*) FROM security_quota_reservations WHERE action='publication') AS quota,(SELECT count(*) FROM security_mutation_results WHERE operation IN ('autonomous_terms_publication','routed_child_publication')) AS results").fetch_one(&store.pool).await.unwrap();
    (
        row.get("terms"),
        row.get("owners"),
        row.get("quota"),
        row.get("results"),
    )
}
async fn publish(
    store: &PostgresStore,
    id: TermsPublicationIdentity<'_>,
    record: &AutonomousBountyTermsRecord,
    attribution: Option<&TermsAttribution>,
    limit: i64,
) -> Result<AutonomousBountyTermsRecord, TermsPublicationError> {
    store
        .publish_owned_terms(OwnedTermsPublication {
            identity: id,
            record,
            daily_limit: limit,
            attribution,
        })
        .await
}
async fn attribution(store: &PostgresStore) -> TermsAttribution {
    let token_hash = hash(&Uuid::new_v4().to_string());
    let acquired = store
        .observe_distribution_acquisition("github", &token_hash, None, Utc::now())
        .await
        .unwrap();
    let handoff = store
        .reserve_distribution_handoff(acquired.id, &hash("fixture handoff"), Utc::now())
        .await
        .unwrap();
    TermsAttribution {
        token_hash,
        handoff_id: handoff.id,
    }
}

#[test]
fn terms_publication_identity_denies_operator_and_binds_operation_and_attribution() {
    let key = hash("key");
    let payload = hash("payload");
    for principal in ["operator", "anonymous", "", "account:bad\nidentity"] {
        assert!(identity_hashes(identity(principal, &key, &payload), None).is_err());
    }
    let id = identity("account:fixture", &key, &payload);
    let ordinary = identity_hashes(id, None).unwrap();
    let mut other = id;
    other.operation = TermsPublicationOperation::RoutedChild;
    assert_ne!(ordinary, identity_hashes(other, None).unwrap());
    let binding = TermsAttribution {
        token_hash: hash("token"),
        handoff_id: Uuid::new_v4(),
    };
    assert_ne!(ordinary.1, identity_hashes(id, Some(&binding)).unwrap().1);
}

#[test]
fn terms_publication_rejects_mismatched_record_hashes_and_creator() {
    let original = record("integrity");
    check_record(&original).unwrap();
    for field in [
        "terms_hash",
        "policy_hash",
        "acceptance_criteria_hash",
        "benchmark_hash",
        "evidence_schema_hash",
        "creator_wallet",
    ] {
        let mut value = serde_json::to_value(&original).unwrap();
        value[field] = json!("0x0000000000000000000000000000000000000000");
        let changed = serde_json::from_value(value).unwrap();
        assert!(
            matches!(check_record(&changed), Err(TermsPublicationError::Conflict)),
            "{field}"
        );
    }
    let mut changed = original.clone();
    changed.document.title = "modified after hashing".to_string();
    assert!(matches!(
        check_record(&changed),
        Err(TermsPublicationError::Conflict)
    ));
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL"]
async fn terms_publication_replay_races_and_historical_ownership_postgres() {
    let (first, second, admin, schema) = stores().await;
    let principal = "account:caller-one";
    let key = hash("same-key");
    let payload = hash("same-request");
    let id = identity(principal, &key, &payload);
    let input = record("parallel same-key");
    let binding = attribution(&first).await;
    let (a, b) = tokio::join!(
        publish(&first, id, &input, Some(&binding), 3),
        publish(&second, id, &input, Some(&binding), 3)
    );
    let saved = a.unwrap();
    assert_eq!(saved, b.unwrap());
    assert_eq!(counts(&first).await, (1, 1, 1, 1));
    assert_eq!(
        first
            .lookup_owned_terms_publication(id, Some(&binding))
            .await
            .unwrap(),
        Some(saved.clone())
    );
    // Durable identity survives the allowance window and original expired cutoff.
    sqlx::query("DELETE FROM security_quota_reservations")
        .execute(&first.pool)
        .await
        .unwrap();
    let mut retry = input.clone();
    retry.created_at = Utc::now();
    assert_eq!(
        publish(&first, id, &retry, Some(&binding), 3)
            .await
            .unwrap(),
        saved
    );
    assert_eq!(counts(&first).await, (1, 1, 0, 1));
    let changed_payload = hash("changed");
    assert!(matches!(
        publish(
            &first,
            identity(principal, &key, &changed_payload),
            &input,
            Some(&binding),
            3
        )
        .await,
        Err(TermsPublicationError::Conflict)
    ));
    assert!(matches!(
        first.lookup_owned_terms_publication(id, None).await,
        Err(TermsPublicationError::Conflict)
    ));
    let changed_binding = attribution(&first).await;
    assert!(matches!(
        first
            .lookup_owned_terms_publication(id, Some(&changed_binding))
            .await,
        Err(TermsPublicationError::Conflict)
    ));
    let new_key = hash("new-key");
    for who in [
        principal,
        "wallet:8453:0x1111111111111111111111111111111111111111",
    ] {
        let result = publish(&second, identity(who, &new_key, &payload), &input, None, 3).await;
        assert!(
            matches!(result,Err(TermsPublicationError::AlreadyExists{terms_hash}) if terms_hash==saved.terms_hash)
        );
    }
    assert_eq!(counts(&first).await, (1, 1, 0, 1));
    let owner: String = sqlx::query_scalar(
        "SELECT principal_key FROM security_resource_owners WHERE resource_id=$1",
    )
    .bind(&saved.terms_hash)
    .fetch_one(&first.pool)
    .await
    .unwrap();
    assert_eq!(owner, principal);
    let unbound: Option<String> =
        sqlx::query_scalar("SELECT terms_hash FROM distribution_acquisition_handoffs WHERE id=$1")
            .bind(changed_binding.handoff_id)
            .fetch_one(&first.pool)
            .await
            .unwrap();
    assert_eq!(unbound, None);
    // Different principals/keys racing one hash get one owner, receipt and allowance.
    let contested = record("cross-principal race");
    let (a, b) = tokio::join!(
        publish(
            &first,
            identity("account:a", &key, &payload),
            &contested,
            None,
            3
        ),
        publish(
            &second,
            identity("account:b", &key, &payload),
            &contested,
            None,
            3
        )
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(
        matches!(a, Err(TermsPublicationError::AlreadyExists { .. }))
            || matches!(b, Err(TermsPublicationError::AlreadyExists { .. }))
    );
    assert_eq!(counts(&first).await, (2, 2, 1, 2));
    let legacy = record("historical raw internal insertion");
    first.upsert_autonomous_bounty_terms(&legacy).await.unwrap();
    assert!(matches!(
        publish(
            &first,
            identity(principal, &new_key, &payload),
            &legacy,
            None,
            3
        )
        .await,
        Err(TermsPublicationError::AlreadyExists { .. })
    ));
    assert_eq!(counts(&first).await, (3, 2, 1, 2));
    let historical = first
        .get_autonomous_bounty_terms(&legacy.terms_hash)
        .await
        .unwrap()
        .unwrap();
    assert!(same_immutable_record(&legacy, &historical).unwrap());
    // Tampered immutable contents are never accepted as a matching stored receipt.
    sqlx::query("UPDATE autonomous_bounty_terms SET policy_hash=$2 WHERE terms_hash=$1")
        .bind(&saved.terms_hash)
        .bind(format!("0x{}", "f".repeat(64)))
        .execute(&first.pool)
        .await
        .unwrap();
    assert!(matches!(
        first
            .lookup_owned_terms_publication(id, Some(&binding))
            .await,
        Err(TermsPublicationError::Conflict)
    ));
    cleanup(first, second, admin, schema).await;
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL"]
async fn terms_publication_quota_shared_and_namespaced_postgres() {
    let (first, second, admin, schema) = stores().await;
    let principal = "account:quota";
    let key = hash("same-external-key");
    let payload = hash("request");
    let trial = NewTrialBounty {
        id: Uuid::new_v4(),
        idempotency_key: hash("scoped trial"),
        request_fingerprint: payload.clone(),
        title: "fixture".into(),
        goal: "fixture".into(),
        acceptance_criteria: vec!["fixture".into()],
        source_url: None,
        discovery_source: "local-test".into(),
        status: "open".into(),
        demo_agent_solution: serde_json::Value::Null,
        expires_at: Utc::now() + Duration::days(1),
    };
    first
        .create_owned_trial_bounty(&trial, principal, &key, 3)
        .await
        .unwrap();
    let id = identity(principal, &key, &payload);
    publish(&first, id, &record("general"), None, 3)
        .await
        .unwrap();
    let mut child = id;
    child.operation = TermsPublicationOperation::RoutedChild;
    publish(&second, child, &record("child"), None, 3)
        .await
        .unwrap();
    assert_eq!(counts(&first).await, (2, 2, 3, 2));
    let fourth_key = hash("fourth");
    assert!(matches!(
        publish(
            &first,
            identity(principal, &fourth_key, &payload),
            &record("denied"),
            None,
            3
        )
        .await,
        Err(TermsPublicationError::Auth(
            auth::AuthStoreError::QuotaExceeded
        ))
    ));
    assert_eq!(counts(&first).await, (2, 2, 3, 2));
    // Even when quota is exhausted, a different key cannot create an alias for an old hash.
    assert!(matches!(
        publish(
            &first,
            identity(principal, &fourth_key, &payload),
            &record("general"),
            None,
            3
        )
        .await,
        Err(TermsPublicationError::AlreadyExists { .. })
    ));
    // Independent pools cannot exceed the final shared allowance in parallel.
    let race_principal = "account:quota-race";
    let left_key = hash("left");
    let right_key = hash("right");
    let left = record("quota race left");
    let right = record("quota race right");
    let (a, b) = tokio::join!(
        publish(
            &first,
            identity(race_principal, &left_key, &payload),
            &left,
            None,
            1
        ),
        publish(
            &second,
            identity(race_principal, &right_key, &payload),
            &right,
            None,
            1
        )
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(
        matches!(
            a,
            Err(TermsPublicationError::Auth(
                auth::AuthStoreError::QuotaExceeded
            ))
        ) || matches!(
            b,
            Err(TermsPublicationError::Auth(
                auth::AuthStoreError::QuotaExceeded
            ))
        )
    );
    assert_eq!(counts(&first).await, (3, 3, 4, 3));
    cleanup(first, second, admin, schema).await;
}

#[tokio::test]
#[ignore = "requires isolated disposable PostgreSQL"]
async fn terms_publication_every_failed_write_rolls_back_postgres() {
    let (first, second, admin, schema) = stores().await;
    let key = hash("key");
    let payload = hash("request");
    let id = identity("account:rollback", &key, &payload);
    let input = record("rollback");
    let binding = attribution(&first).await;
    let wrong = TermsAttribution {
        token_hash: hash("not-owner-token"),
        handoff_id: binding.handoff_id,
    };
    assert!(matches!(
        publish(&first, id, &input, Some(&wrong), 3).await,
        Err(TermsPublicationError::AttributionConflict)
    ));
    assert_eq!(counts(&first).await, (0, 0, 0, 0));
    for table in [
        "autonomous_bounty_terms",
        "security_resource_owners",
        "security_mutation_results",
    ] {
        sqlx::query("CREATE FUNCTION fail_publication_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'isolated expected write failure'; END; $$").execute(&first.pool).await.unwrap();
        sqlx::query(&format!("CREATE TRIGGER reject_publication_insert BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION fail_publication_insert()" )).execute(&first.pool).await.unwrap();
        assert!(publish(&first, id, &input, Some(&binding), 3)
            .await
            .is_err());
        assert_eq!(
            counts(&first).await,
            (0, 0, 0, 0),
            "{table} must roll back every effect"
        );
        let value: Option<String> = sqlx::query_scalar(
            "SELECT terms_hash FROM distribution_acquisition_handoffs WHERE id=$1",
        )
        .bind(binding.handoff_id)
        .fetch_one(&first.pool)
        .await
        .unwrap();
        assert_eq!(
            value, None,
            "attribution must roll back after {table} failure"
        );
        sqlx::query(&format!(
            "DROP TRIGGER reject_publication_insert ON {table}"
        ))
        .execute(&first.pool)
        .await
        .unwrap();
        sqlx::query("DROP FUNCTION fail_publication_insert()")
            .execute(&first.pool)
            .await
            .unwrap();
    }
    let saved = publish(&first, id, &input, Some(&binding), 3)
        .await
        .unwrap();
    assert_eq!(counts(&first).await, (1, 1, 1, 1));
    // Existing callers retain the original immutable attribution semantics.
    let original = first
        .bind_distribution_handoff_terms(
            &binding.token_hash,
            binding.handoff_id,
            &saved.terms_hash,
            &saved.creator_wallet,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        original.terms_hash.as_deref(),
        Some(saved.terms_hash.as_str())
    );
    let other = record("binding conflict");
    let other_key = hash("different key");
    assert!(matches!(
        publish(
            &second,
            identity(id.principal, &other_key, &payload),
            &other,
            Some(&binding),
            3
        )
        .await,
        Err(TermsPublicationError::AttributionConflict)
    ));
    assert_eq!(counts(&first).await, (1, 1, 1, 1));
    cleanup(first, second, admin, schema).await;
}
