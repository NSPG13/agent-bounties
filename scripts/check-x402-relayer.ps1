$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$existingPath = $env:Path
[Environment]::SetEnvironmentVariable("PATH", $null, [EnvironmentVariableTarget]::Process)
[Environment]::SetEnvironmentVariable("Path", $null, [EnvironmentVariableTarget]::Process)
[Environment]::SetEnvironmentVariable(
    "Path",
    "$repoRoot\.tools\foundry;$existingPath",
    [EnvironmentVariableTarget]::Process
)
$port = if ($env:X402_RELAYER_TEST_PORT) { $env:X402_RELAYER_TEST_PORT } else { "18545" }
$rpcUrl = "http://127.0.0.1:$port"
$logDirectory = Join-Path $repoRoot "target\tmp"
$stdout = Join-Path $logDirectory "x402-relayer-anvil.stdout.log"
$stderr = Join-Path $logDirectory "x402-relayer-anvil.stderr.log"
New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null

$anvil = Get-Command anvil -ErrorAction Stop
$process = Start-Process -FilePath $anvil.Source `
    -ArgumentList @("--silent", "--port", $port, "--chain-id", "31337") `
    -RedirectStandardOutput $stdout `
    -RedirectStandardError $stderr `
    -WindowStyle Hidden `
    -PassThru
try {
    $ready = $false
    for ($i = 0; $i -lt 30; $i++) {
        & cast block-number --rpc-url $rpcUrl 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) {
            $ready = $true
            break
        }
        Start-Sleep -Seconds 1
    }
    if (-not $ready) {
        throw "Anvil did not become ready; see $stderr"
    }
    $previousRpc = $env:AGENT_BOUNTIES_TEST_RPC_URL
    $env:AGENT_BOUNTIES_TEST_RPC_URL = $rpcUrl
    try {
        cargo test -p chain-base `
            tests::hosted_relayer_rehearsal_broadcasts_bounded_zero_value_transaction `
            -- --ignored --exact --nocapture
        if ($LASTEXITCODE -ne 0) {
            throw "x402 relayer rehearsal failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        $env:AGENT_BOUNTIES_TEST_RPC_URL = $previousRpc
    }
}
finally {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
}
