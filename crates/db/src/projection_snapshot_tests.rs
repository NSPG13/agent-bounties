use super::*;

async fn wait_for_blocked_read(pool: &PgPool, table: &str) {
    for _ in 0..100 {
        let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event='relation' AND query LIKE $1)")
            .bind(format!("%FROM {table}%")).fetch_one(pool).await.unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("snapshot did not reach the deliberately locked data table");
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn projection_snapshot_cannot_mix_new_rows_with_old_certification_postgres() {
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    crate::test_support::assert_isolated_security_database(&url);
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("projection_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPool::connect_with(options).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.unwrap();
    let factory = "0x1111111111111111111111111111111111111111";
    let child = "0x2222222222222222222222222222222222222222";
    let network = "base-mainnet";
    store
        .upsert_base_log_cursor(network, factory, 100, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO autonomous_canonical_checkpoints(network,factory_contract,block_number,block_hash) VALUES($1,$2,100,$3)")
        .bind(network).bind(factory).bind(format!("0x{}", "ab".repeat(32))).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO autonomous_bounty_events(id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data) VALUES($1,'old',$2,'tx-old',90,0,$3,'bounty','canonical_bounty_created',$4)")
        .bind(Uuid::new_v4()).bind(network).bind(factory).bind(serde_json::json!({"bounty_contract":child})).execute(&pool).await.unwrap();
    let mut writer = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE autonomous_bounty_events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *writer)
        .await
        .unwrap();
    let read_store = store.clone();
    let reading = tokio::spawn(async move {
        read_store
            .autonomous_projection_snapshot(network, factory, None, 100)
            .await
    });
    wait_for_blocked_read(&pool, "autonomous_bounty_events").await;
    sqlx::query("UPDATE autonomous_canonical_checkpoints SET invalidated=true WHERE network=$1 AND factory_contract=$2")
        .bind(network).bind(factory).execute(&mut *writer).await.unwrap();
    sqlx::query("INSERT INTO autonomous_bounty_events(id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data) VALUES($1,'new',$2,'tx-new',95,0,$3,'bounty','funding_added','{}')")
        .bind(Uuid::new_v4()).bind(network).bind(child).execute(&mut *writer).await.unwrap();
    writer.commit().await.unwrap();
    let old = reading.await.unwrap().unwrap();
    assert_eq!(
        old.events.len(),
        1,
        "new rows cannot borrow old certification"
    );
    assert_eq!(old.events[0].log_key, "old");
    assert!(
        store
            .autonomous_projection_snapshot(network, factory, None, 100)
            .await
            .is_err(),
        "a new complete snapshot must observe quarantine"
    );
    assert!(
        store
            .projection_reader_snapshot(network, factory)
            .await
            .is_err(),
        "quarantine cannot become wait-eligible"
    );

    let beta = "0x3333333333333333333333333333333333333333";
    sqlx::query("INSERT INTO open_competition_v2_indexer_agreements(network,factory_contract,protocol_version,common_safe_block,primary_safe_head,shadow_safe_head,primary_block_hash,shadow_block_hash,canonical_event_count,canonical_event_set_hash,agrees,observed_at) VALUES($1,$2,'agent-bounties/open-competition-v2-beta3',100,100,100,'same','same',0,'empty',true,now())")
        .bind(network).bind(beta).execute(&pool).await.unwrap();
    let mut writer = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE open_competition_v2_projections IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *writer)
        .await
        .unwrap();
    let read_store = store.clone();
    let reading = tokio::spawn(async move {
        read_store
            .open_competition_v2_projection_snapshot(network, beta, None, 100)
            .await
    });
    wait_for_blocked_read(&pool, "open_competition_v2_projections").await;
    sqlx::query("UPDATE open_competition_v2_indexer_agreements SET agrees=false,failure_code='canonical_mismatch' WHERE network=$1 AND factory_contract=$2")
        .bind(network).bind(beta).execute(&mut *writer).await.unwrap();
    sqlx::query("INSERT INTO open_competition_v2_projections(network,factory_contract,bounty_id,competition_contract,creator,state,solver_reward,keeper_reward,funded_amount,accepted_entries,refund_pool_remaining,last_block,last_log_index,safe_block_number,safe_block_hash) VALUES($1,$2,'new',$3,$3,'funding','100','0','0',0,'0',101,0,101,'hash')")
        .bind(network).bind(beta).bind(child).execute(&mut *writer).await.unwrap();
    writer.commit().await.unwrap();
    let old = reading.await.unwrap().unwrap();
    assert!(old.agreement.agrees);
    assert!(
        old.records.is_empty(),
        "newer projection rows cannot borrow an old agreement"
    );
    assert!(store
        .open_competition_v2_projection_snapshot(network, beta, None, 100)
        .await
        .is_err());
    assert!(store
        .projection_reader_snapshot(network, beta)
        .await
        .is_err());
    sqlx::query("UPDATE open_competition_v2_indexer_agreements SET agrees=true,failure_code=NULL WHERE network=$1 AND factory_contract=$2")
        .bind(network).bind(beta).execute(&pool).await.unwrap();
    assert!(
        store
            .open_competition_v2_projection_snapshot(network, beta, None, 100)
            .await
            .is_err(),
        "a newer current row must fail, not disappear as an empty source"
    );
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}
