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
    Invoke-Checked { cargo run -p cli -- demo }
}
finally {
    Pop-Location
}
