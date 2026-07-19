$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock] $Command
    )

    $global:LASTEXITCODE = 0
    & $Command
    $commandSucceeded = $?
    $exitCode = $global:LASTEXITCODE
    if (-not $commandSucceeded -or $exitCode -ne 0) {
        throw "Command failed with exit code $exitCode`: $Command"
    }
    $global:LASTEXITCODE = 0
}

Push-Location $repoRoot
try {
    Invoke-Checked { docker compose up -d postgres }

    $ready = $false
    for ($i = 0; $i -lt 30; $i++) {
        docker compose exec -T postgres pg_isready -U agent_bounties | Out-Null
        if ($LASTEXITCODE -eq 0) {
            $ready = $true
            break
        }
        Start-Sleep -Seconds 2
    }

    if (-not $ready) {
        throw "Postgres did not become ready within 60 seconds"
    }

    $databaseUrl = $env:DATABASE_URL
    if (-not $databaseUrl) {
        $databaseUrl = "postgres://agent_bounties:agent_bounties@localhost:5432/agent_bounties"
    }

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
