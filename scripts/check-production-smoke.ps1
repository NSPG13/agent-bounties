param(
    [string] $ApiBaseUrl = $env:PRODUCTION_API_BASE_URL,
    [string] $McpBaseUrl = $env:PRODUCTION_MCP_BASE_URL,
    [string] $ExpectedRevision = $env:PRODUCTION_EXPECTED_REVISION,
    [switch] $RequireEvalHistory
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($ApiBaseUrl)) {
    throw "Set PRODUCTION_API_BASE_URL or pass -ApiBaseUrl."
}
if ([string]::IsNullOrWhiteSpace($McpBaseUrl)) {
    throw "Set PRODUCTION_MCP_BASE_URL or pass -McpBaseUrl."
}

$env:Path = "$env:USERPROFILE\.cargo\bin;$repoRoot\.tools\foundry;$env:Path"
$args = @(
    "run", "-p", "cli", "--",
    "production-smoke",
    "--api-base-url", $ApiBaseUrl,
    "--mcp-base-url", $McpBaseUrl
)
if ($RequireEvalHistory) {
    $args += "--require-eval-history"
}
if (-not [string]::IsNullOrWhiteSpace($ExpectedRevision)) {
    $args += @("--expected-revision", $ExpectedRevision)
}

Push-Location $repoRoot
try {
    cargo @args
    if ($LASTEXITCODE -ne 0) {
        throw "production smoke failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}
