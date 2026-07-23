$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
. (Join-Path $PSScriptRoot "_shared\powershell.ps1")

Push-Location $repoRoot
try {
    Start-AgentBountiesPostgres
    $env:DATABASE_URL = Get-AgentBountiesDatabaseUrl

    Invoke-Checked { cargo run -p api }
}
finally {
    Pop-Location
}
