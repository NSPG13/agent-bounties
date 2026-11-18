use super::*;

#[tokio::test]
#[ignore = "requires isolated local AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn recovery_connections_reject_durable_writes_on_new_and_reused_sessions() {
    assert_eq!(
        std::env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref(),
        Ok("1")
    );
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    crate::test_support::assert_isolated_security_database(&url);
    let admin = PostgresStore::connect(&url).await.unwrap();
    let name = format!("recovery_read_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!(
        "CREATE TABLE {name} (id integer PRIMARY KEY, value text NOT NULL)"
    ))
    .execute(&admin.pool)
    .await
    .unwrap();
    sqlx::query(&format!("INSERT INTO {name} VALUES (1, 'original')"))
        .execute(&admin.pool)
        .await
        .unwrap();
    let reader = PostgresStore::connect_read_only(&url).await.unwrap();
    let mut connections = Vec::new();
    for _ in 0..4 {
        let mut connection = reader.pool.acquire().await.unwrap();
        let mode: String = sqlx::query_scalar("SHOW transaction_read_only")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(mode, "on");
        connections.push(connection);
    }
    drop(connections);
    for statement in [
        format!("INSERT INTO {name} VALUES (2, 'changed')"),
        format!("UPDATE {name} SET value = 'changed'"),
        format!("DELETE FROM {name}"),
        format!("TRUNCATE {name}"),
        format!("ALTER TABLE {name} ADD COLUMN extra text"),
        format!("CREATE TABLE {name}_other (id integer)"),
    ] {
        let error = sqlx::query(&statement)
            .execute(&reader.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("25006")
        );
    }
    // The role is deliberately write-capable in this fixture. Session policy
    // is restored on reuse even if an internal borrower changed its default.
    let mut connection = reader.pool.acquire().await.unwrap();
    sqlx::query("SET default_transaction_read_only = off")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    for _ in 0..8 {
        let mode: String = sqlx::query_scalar("SHOW transaction_read_only")
            .fetch_one(&reader.pool)
            .await
            .unwrap();
        assert_eq!(mode, "on");
    }
    let mut transaction = reader.pool.begin().await.unwrap();
    let error = sqlx::query(&format!("UPDATE {name} SET value = 'transaction'"))
        .execute(&mut *transaction)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("25006")
    );
    transaction.rollback().await.unwrap();
    let values: Vec<String> = sqlx::query_scalar(&format!("SELECT value FROM {name}"))
        .fetch_all(&admin.pool)
        .await
        .unwrap();
    assert_eq!(values, ["original"]);
    sqlx::query(&format!("DROP TABLE {name}"))
        .execute(&admin.pool)
        .await
        .unwrap();
}
