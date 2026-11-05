param(
    [string] $ExternalDatabaseUrl = ""
)

$ErrorActionPreference = "Stop"
$python = Get-Command python -ErrorAction SilentlyContinue
$pythonArgs = @()
if (-not $python) { $python = Get-Command py -ErrorAction SilentlyContinue; $pythonArgs = @("-3") }
if (-not $python) { throw "python or py is required for the PostgreSQL test guard" }
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
. (Join-Path $PSScriptRoot "_shared\powershell.ps1")

Push-Location $repoRoot
try {
    if ($ExternalDatabaseUrl) {
        $databaseUrl = $ExternalDatabaseUrl
    }
    else {
        Start-AgentBountiesPostgres
        $databaseUrl = Get-AgentBountiesDatabaseUrl
    }

    Invoke-Checked { cargo build -p api -p mcp-server }
    $previousTestDatabaseUrl = $env:AGENT_BOUNTIES_TEST_DATABASE_URL
    $env:AGENT_BOUNTIES_TEST_DATABASE_URL = $databaseUrl
    try {
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::x402_relay_attempt_is_idempotent_and_lease_bounded --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::claim_funnel_counts_direct_and_atomic_sponsored_confirmations --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::opportunity_lifecycle_query_executes_against_migrated_postgres --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::site_analytics_round_trip_executes_against_migrated_postgres --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::platform_metrics_query_separates_history_from_identity_and_cohort_boundaries --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::controlled_card_test_exclusion_preserves_money_and_shared_wallet_customers --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db managed_service_orders::metrics::postgres_tests:: --test-threads=1
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api platform_payouts::tests::platform_payout_http_replays_real_database_selection --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::opportunity_comment_round_trip_is_durable_and_idempotent --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::chatgpt_action_intent_replays_and_confirms_only_observed_transaction --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db site_posting_drafts
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::social_mention_ingestion_round_trip_executes_against_migrated_postgres --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::objective_aggregate_compare_and_swap_is_durable --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::discovery_webhook_round_trip_executes_against_migrated_postgres --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::autonomous_v2_events_round_trip_through_postgres --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db managed_service_orders::tests:: --test-threads=1
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api managed_payments::tests:: --test-threads=1
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db tests::discoverability_snapshot_idempotency_and_restart_hydration_are_durable --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api open_competition_v2_api::tests::postgres_readiness_hold_blocks_new_payments_but_preserves_reconciliation --exact --test-threads=1
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api tests::audience_audit_persists_idempotently_across_processes --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api tests::github_issue_api_sync_postgres_rejects_stale_cross_process_activity --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api tests::github_issue_api_sync_postgres_serializes_concurrent_initial_sync --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api tests::neynar_webhook_persists_one_short_draft_and_one_reply_across_retries --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api tests::bounty_status_reads_scoped_postgres_after_cross_process_funding --exact
        }
        Invoke-Checked {
            & $python.Source @pythonArgs scripts/run-required-postgres-test.py -p mcp-server tests::mcp_bounty_status_reads_scoped_postgres_after_cross_process_funding --exact
        }
    }
    finally {
        $env:AGENT_BOUNTIES_TEST_DATABASE_URL = $previousTestDatabaseUrl
    }
    # Security regressions always use their own explicit local fixture.
    Invoke-Checked {
        & $python.Source @pythonArgs scripts/run-with-test-postgres.py -- $python.Source @pythonArgs scripts/run-required-postgres-test.py -p db projection_snapshot::tests::projection_snapshot_cannot_mix_new_rows_with_old_certification_postgres --exact --test-threads=1
    }
    Invoke-Checked {
        & $python.Source @pythonArgs scripts/run-with-test-postgres.py -- $python.Source @pythonArgs scripts/run-required-postgres-test.py -p api marketplace_readiness::progress_tests::progress_shared_observation_uses_four_rpc_calls_with_real_db_progress_postgres --exact --test-threads=1
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
