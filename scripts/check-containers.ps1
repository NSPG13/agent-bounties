param(
    [string] $ApiImage = "agent-bounties-api:local",
    [string] $McpImage = "agent-bounties-mcp:local",
    [string] $WorkerImage = "agent-bounties-worker:local"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "_shared\powershell.ps1")

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
