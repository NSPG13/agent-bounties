param(
    [string] $ProjectName = "agent-bounties-prod-smoke",
    [string] $EnvFile = ".env.example",
    [int] $ApiPort = 18080,
    [int] $McpPort = 18090,
    [switch] $RequireEvalHistory
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot

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

function Wait-HttpOk {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Url
    )

    for ($i = 0; $i -lt 60; $i++) {
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $Url -TimeoutSec 2 | Out-Null
            return
        } catch {
            Start-Sleep -Seconds 2
        }
    }

    throw "$Url did not become healthy within 120 seconds"
}

Push-Location $repoRoot
$oldApiPort = $env:API_PORT
$oldMcpPort = $env:MCP_PORT
$oldPublicBaseUrl = $env:PUBLIC_BASE_URL
$oldMcpBaseUrl = $env:MCP_BASE_URL
$env:API_PORT = "$ApiPort"
$env:MCP_PORT = "$McpPort"
$env:PUBLIC_BASE_URL = "http://127.0.0.1:$ApiPort"
$env:MCP_BASE_URL = "http://127.0.0.1:$McpPort"
$composeArgs = @(
    "--env-file", $EnvFile,
    "-p", $ProjectName,
    "-f", "docker-compose.production.yml"
)

try {
    Invoke-Checked { docker compose @composeArgs up -d --build }
    Wait-HttpOk "http://127.0.0.1:$ApiPort/health"
    Wait-HttpOk "http://127.0.0.1:$McpPort/health"

    Invoke-Checked {
        if ($RequireEvalHistory) {
            & "$PSScriptRoot\check-production-smoke.ps1" `
                -ApiBaseUrl "http://127.0.0.1:$ApiPort" `
                -McpBaseUrl "http://127.0.0.1:$McpPort" `
                -RequireEvalHistory
        }
        else {
            & "$PSScriptRoot\check-production-smoke.ps1" `
                -ApiBaseUrl "http://127.0.0.1:$ApiPort" `
                -McpBaseUrl "http://127.0.0.1:$McpPort"
        }
    }
}
finally {
    docker compose @composeArgs down -v
    $env:API_PORT = $oldApiPort
    $env:MCP_PORT = $oldMcpPort
    $env:PUBLIC_BASE_URL = $oldPublicBaseUrl
    $env:MCP_BASE_URL = $oldMcpBaseUrl
    Pop-Location
}
