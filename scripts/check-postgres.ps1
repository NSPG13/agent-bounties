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
            cargo test -p api bounty_status_reads_base_events_from_postgres_after_cross_process_indexing -- --ignored --exact
        }
        Invoke-Checked {
            cargo test -p mcp-server mcp_bounty_status_reads_scoped_postgres_after_cross_process_funding -- --ignored --exact
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
