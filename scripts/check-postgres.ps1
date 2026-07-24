$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
. (Join-Path $PSScriptRoot "_shared\powershell.ps1")

Push-Location $repoRoot
try {
    Start-AgentBountiesPostgres
    $databaseUrl = Get-AgentBountiesDatabaseUrl

    Invoke-Checked { cargo build -p api -p mcp-server }
    $previousTestDatabaseUrl = $env:AGENT_BOUNTIES_TEST_DATABASE_URL
    $env:AGENT_BOUNTIES_TEST_DATABASE_URL = $databaseUrl
    try {
        Invoke-Checked {
            cargo test -p db tests::x402_relay_attempt_is_idempotent_and_lease_bounded -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p db tests::claim_funnel_counts_direct_and_atomic_sponsored_confirmations -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p db tests::opportunity_lifecycle_query_executes_against_migrated_postgres -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p db tests::site_analytics_round_trip_executes_against_migrated_postgres -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p db tests::social_mention_ingestion_round_trip_executes_against_migrated_postgres -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p db tests::competitor_intelligence_migration_executes_against_migrated_postgres -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p db tests::objective_aggregate_compare_and_swap_is_durable -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p db tests::discovery_webhook_round_trip_executes_against_migrated_postgres -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p api tests::audience_audit_persists_idempotently_across_processes -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p api tests::github_issue_api_sync_postgres_rejects_stale_cross_process_activity -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p api tests::github_issue_api_sync_postgres_serializes_concurrent_initial_sync -- --ignored --exact --nocapture
        }
        Invoke-Checked {
            cargo test -p api tests::neynar_webhook_persists_one_short_draft_and_one_reply_across_retries -- --ignored --exact --nocapture
        }
    }
    finally {
        $env:AGENT_BOUNTIES_TEST_DATABASE_URL = $previousTestDatabaseUrl
    }
    Invoke-Checked {
        cargo run -p cli -- service-smoke-spawn `
            --api-base-url http://127.0.0.1:18180 `
            --mcp-base-url http://127.0.0.1:18190 `
            --database-url $databaseUrl `
            --verify-restart-persistence
    }
}
finally {
    Pop-Location
}
