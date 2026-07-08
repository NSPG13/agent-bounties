param(
    [string] $ApiImage = "agent-bounties-api:local",
    [string] $McpImage = "agent-bounties-mcp:local",
    [string] $WorkerImage = "agent-bounties-worker:local"
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

Push-Location $repoRoot
try {
    Invoke-Checked {
        docker build `
            --build-arg APP_PACKAGE=api `
            --build-arg APP_BINARY=api `
            -t $ApiImage `
            .
    }
    Invoke-Checked {
        docker build `
            --build-arg APP_PACKAGE=mcp-server `
            --build-arg APP_BINARY=mcp-server `
            -t $McpImage `
            .
    }
    Invoke-Checked {
        docker build `
            --build-arg APP_PACKAGE=worker `
            --build-arg APP_BINARY=worker `
            -t $WorkerImage `
            .
    }
} finally {
    Pop-Location
}
