//! Guards for tests that deliberately write to a disposable local database.

use sqlx::postgres::PgConnectOptions;
use std::str::FromStr;

/// Reject production URLs before a security regression test can connect.
pub fn assert_isolated_security_database(url: &str) {
    let options = PgConnectOptions::from_str(url).expect("valid isolated test database URL");
    let host = options.get_host();
    let database = options.get_database().unwrap_or_default();
    let marked_fixture = std::env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref() == Ok("1")
        && options.get_username() == "agent_bounties_test"
        && database == "agent_bounties_test";
    let investigation_fixture = options.get_port() == 15483
        && options.get_username() == "postgres"
        && database.starts_with("security_auth_");
    assert!(
        matches!(host, "127.0.0.1" | "localhost") && (marked_fixture || investigation_fixture),
        "security regressions require an explicitly isolated local database"
    );
}
