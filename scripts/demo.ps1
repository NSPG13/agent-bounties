$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
. (Join-Path $PSScriptRoot "_shared\powershell.ps1")

Push-Location $repoRoot
try {
    Invoke-Checked { cargo run -p cli -- demo }
}
finally {
    Pop-Location
}
