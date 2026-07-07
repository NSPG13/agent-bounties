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

    if (-not $env:DATABASE_URL) {
        $env:DATABASE_URL = "postgres://agent_bounties:agent_bounties@localhost:5432/agent_bounties"
    }

    Invoke-Checked { cargo run -p mcp-server }
}
finally {
    Pop-Location
}
