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

    $fundingJson = cargo run -q -p cli -- funding-rehearsal-demo | Out-String
    [System.IO.File]::WriteAllText(
        (Join-Path $resolvedOutDir "funding-rehearsal-demo.json"),
        $fundingJson,
        $utf8NoBom
    )

    $readinessJson = cargo run -q -p cli -- real-funding-readiness `
        --network base-sepolia `
        --escrow-contract 0x1111111111111111111111111111111111111111 `
        --usdc-token 0x3333333333333333333333333333333333333333 |
        Out-String
    [System.IO.File]::WriteAllText(
        (Join-Path $resolvedOutDir "real-funding-readiness.json"),
        $readinessJson,
        $utf8NoBom
    )

    & $pythonCommand.Source @pythonArgs scripts\validate_real_funding_rehearsal.py $OutDir
}
finally {
    Pop-Location
}
