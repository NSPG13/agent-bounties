param(
    [ValidateSet("core", "full")]
    [string] $Mode = "core"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$repoRoot\.tools\foundry;$env:Path"
$failures = New-Object System.Collections.Generic.List[string]
$warnings = New-Object System.Collections.Generic.List[string]
$minimumRustMajor = 1
$minimumRustMinor = 88

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

function Test-MinimumVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Name,
        [Parameter(Mandatory = $true)]
        [int] $MinimumMajor,
        [Parameter(Mandatory = $true)]
        [int] $MinimumMinor,
        [Parameter(Mandatory = $true)]
        [string] $Purpose
    )

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        return
    }

    $previousErrorActionPreference = $ErrorActionPreference
    try {
        # Windows PowerShell can promote native stderr warnings to terminating
        # errors even when the command exits successfully. Version checks care
        # about the exit code and stdout, so handle those explicitly.
        $ErrorActionPreference = "SilentlyContinue"
        $versionOutput = & $Name --version 2>$null
        $versionExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    if ($versionExitCode -ne 0) {
        $failures.Add("$Name --version failed for $Purpose with exit code $versionExitCode")
        return
    }

    if ($versionOutput -notmatch '^\S+\s+(\d+)\.(\d+)\.') {
        $failures.Add("could not parse $Name version for $Purpose")
        return
    }

    $major = [int] $Matches[1]
    $minor = [int] $Matches[2]
    if ($major -lt $MinimumMajor -or ($major -eq $MinimumMajor -and $minor -lt $MinimumMinor)) {
        $failures.Add("$Name $MinimumMajor.$MinimumMinor or newer is required for $Purpose; found $versionOutput")
    }
}

Test-RequiredCommand rustc "Rust compiler commands"
Test-RequiredCommand cargo "Rust workspace commands"
Test-RequiredCommand npm "TypeScript SDK checks"
Test-MinimumVersion rustc $minimumRustMajor $minimumRustMinor "the locked dependency graph"
Test-MinimumVersion cargo $minimumRustMajor $minimumRustMinor "the locked dependency graph"

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
$driveRoot = (Get-Item $repoRoot).PSDrive.Root
try {
    $freeBytes = [System.IO.DriveInfo]::new($driveRoot).AvailableFreeSpace
}
catch {
    $freeBytes = (Get-PSDrive -Name $driveName).Free
}
$freeMb = [math]::Floor($freeBytes / 1MB)
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
