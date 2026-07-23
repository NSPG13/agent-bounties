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

function Start-AgentBountiesPostgres {
    Invoke-Checked { docker compose up -d postgres }

    for ($i = 0; $i -lt 30; $i++) {
        docker compose exec -T postgres pg_isready -U agent_bounties | Out-Null
        if ($LASTEXITCODE -eq 0) {
            return
        }
        Start-Sleep -Seconds 2
    }

    throw "Postgres did not become ready within 60 seconds"
}

function Get-AgentBountiesDatabaseUrl {
    if ($env:DATABASE_URL) {
        return $env:DATABASE_URL
    }
    return "postgres://agent_bounties:agent_bounties@localhost:5432/agent_bounties"
}
