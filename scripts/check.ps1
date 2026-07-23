$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$repoRoot\.tools\foundry;$env:Path"
$python = Get-Command python -ErrorAction SilentlyContinue
$pythonArgs = @()
if (-not $python) { $python = Get-Command py -ErrorAction SilentlyContinue; $pythonArgs = @("-3") }
if (-not $python) { throw "python or py is required to compile the Python SDK" }
Push-Location $repoRoot
try { & $python.Source @pythonArgs scripts\check.py --platform powershell; if ($LASTEXITCODE) { exit $LASTEXITCODE } }
finally { Pop-Location }
