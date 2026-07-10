param(
    [string] $OutDir = "target\real-funding-rehearsal"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

$pythonCommand = Get-Command python -ErrorAction SilentlyContinue
$pythonArgs = @()
if (-not $pythonCommand) {
    $pythonCommand = Get-Command py -ErrorAction SilentlyContinue
    $pythonArgs = @("-3")
}
if (-not $pythonCommand) {
    throw "python or py is required to validate real funding rehearsal artifacts"
}

Push-Location $repoRoot
try {
    New-Item -ItemType Directory -Force $OutDir | Out-Null
    $resolvedOutDir = (Resolve-Path $OutDir).Path
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)

    $discoveryJson = cargo run -q -p cli -- discovery `
        --public-base-url https://agentbounties.example `
        --mcp-base-url https://agentbounties.example/mcp |
        Out-String
    [System.IO.File]::WriteAllText(
        (Join-Path $resolvedOutDir "autonomous-discovery.json"),
        $discoveryJson,
        $utf8NoBom
    )

    $readinessJson = cargo run -q -p cli -- real-funding-readiness `
        --network base-sepolia `
        --escrow-contract 0x1111111111111111111111111111111111111111 `
        --usdc-token 0x036CbD53842c5426634e7929541eC2318f3dCF7e |
        Out-String
    [System.IO.File]::WriteAllText(
        (Join-Path $resolvedOutDir "autonomous-readiness.json"),
        $readinessJson,
        $utf8NoBom
    )
    Copy-Item deployments\base-mainnet.json `
        (Join-Path $resolvedOutDir "base-mainnet-deployment.json") -Force

    & $pythonCommand.Source @pythonArgs scripts\validate_real_funding_rehearsal.py $OutDir
}
finally {
    Pop-Location
}
