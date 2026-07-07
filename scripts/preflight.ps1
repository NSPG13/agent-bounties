param(
    [ValidateSet("core", "full")]
    [string] $Mode = "core"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$repoRoot\.tools\foundry;$env:Path"
$failures = New-Object System.Collections.Generic.List[string]
$warnings = New-Object System.Collections.Generic.List[string]

function Test-RequiredCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Name,
        [Parameter(Mandatory = $true)]
        [string] $Purpose
    )

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        $failures.Add("$Name is required for $Purpose")
    }
}

function Test-OptionalCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Name,
        [Parameter(Mandatory = $true)]
        [string] $Purpose
    )

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        $warnings.Add("$Name is optional; install it for $Purpose")
    }
}

Test-RequiredCommand cargo "Rust workspace commands"
Test-RequiredCommand npm "TypeScript SDK checks"

$pythonCommand = Get-Command python -ErrorAction SilentlyContinue
if (-not $pythonCommand) {
    $pythonCommand = Get-Command py -ErrorAction SilentlyContinue
}
if (-not $pythonCommand) {
    $failures.Add("python or py is required for Python SDK checks")
}

if ($Mode -eq "full") {
    Test-RequiredCommand forge "Base escrow contract tests"
    $minimumFreeMb = 4096
}
else {
    $minimumFreeMb = 512
}

Test-OptionalCommand docker "Postgres durability smoke tests"

$driveName = (Get-Item $repoRoot).PSDrive.Name
$drive = Get-PSDrive -Name $driveName
$freeMb = [math]::Floor($drive.Free / 1MB)
if ($freeMb -lt $minimumFreeMb) {
    $failures.Add("free disk on drive $driveName is ${freeMb}MB; $Mode mode expects at least ${minimumFreeMb}MB. If this is a development checkout, run cargo clean to remove generated target output.")
}

if ($warnings.Count -gt 0) {
    foreach ($warning in $warnings) {
        Write-Warning $warning
    }
}

if ($failures.Count -gt 0) {
    Write-Error ("Preflight failed:`n- " + ($failures -join "`n- "))
}

Write-Host "preflight=$Mode ok free_mb=$freeMb"
