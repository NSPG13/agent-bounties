param(
    [string]$Repository = "NSPG13/agent-bounties",
    [string]$ApiBaseUrl = "http://127.0.0.1:8080",
    [string]$OperatorToken = $env:OPERATOR_API_TOKEN,
    [int]$Limit = 200,
    [switch]$IncludeOwner
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command py -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "Python is required."
}

Write-Warning "This compatibility wrapper now imports public GitHub participation into the consent-aware audience registry. It does not add inferred contacts, email addresses, or wallets."
if ($Limit -ne 200) {
    Write-Warning "-Limit is retained for compatibility but ignored; the idempotent audit paginates all public repository activity."
}

$output = Join-Path $repoRoot "target\github-audience-audit.json"
$arguments = @(
    (Join-Path $PSScriptRoot "github_audience_audit.py"),
    "--repository", $Repository,
    "--output", $output,
    "--sync",
    "--api-base-url", $ApiBaseUrl
)
if ($IncludeOwner) {
    $arguments += "--include-owner"
}

$previousToken = $env:OPERATOR_API_TOKEN
try {
    if (-not [string]::IsNullOrWhiteSpace($OperatorToken)) {
        $env:OPERATOR_API_TOKEN = $OperatorToken
    }
    & $python.Source @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "GitHub audience audit exited with code $LASTEXITCODE"
    }
}
finally {
    $env:OPERATOR_API_TOKEN = $previousToken
}
