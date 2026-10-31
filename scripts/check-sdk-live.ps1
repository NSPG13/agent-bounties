$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
. (Join-Path $PSScriptRoot "_shared\powershell.ps1")

$pythonCommand = Get-Command python -ErrorAction SilentlyContinue
$pythonArgs = @()
if (-not $pythonCommand) {
    $pythonCommand = Get-Command py -ErrorAction SilentlyContinue
    $pythonArgs = @("-3")
}
if (-not $pythonCommand) {
    throw "python or py is required to run the Python SDK smoke"
}

if ($env:AGENT_BOUNTIES_LOCAL_TEST_DATABASE -ne "1") {
    $shellPath = (Get-Process -Id $PID).Path
    Invoke-Checked { & $pythonCommand.Source @pythonArgs (Join-Path $PSScriptRoot "run-with-test-postgres.py") -- $shellPath -NoProfile -File $PSCommandPath }
    return
}
Invoke-Checked { & $pythonCommand.Source @pythonArgs (Join-Path $PSScriptRoot "run-with-test-postgres.py") --validate-existing }

& $pythonCommand.Source @pythonArgs -c "import httpx; from eth_hash.auto import keccak" *> $null
if ($LASTEXITCODE -ne 0) {
    Invoke-Checked { & $pythonCommand.Source @pythonArgs -m pip install -e (Join-Path $repoRoot "crates\sdk-python") }
}
$global:LASTEXITCODE = 0

Push-Location $repoRoot
try {
    Invoke-Checked { cargo build -p api }

    $apiBaseUrl = "http://127.0.0.1:18280"
    $oldApiBind = $env:API_BIND_ADDR
    $oldPublicBase = $env:PUBLIC_BASE_URL
    $oldMcpBase = $env:MCP_BASE_URL
    $oldDatabaseUrl = $env:DATABASE_URL
    $oldPythonPath = $env:PYTHONPATH
    $oldOperatorApiToken = $env:OPERATOR_API_TOKEN
    $oldWalletSession = $env:AGENT_BOUNTIES_SMOKE_WALLET_SESSION
    $oldPublicationLimit = $env:AUTH_PUBLICATIONS_PER_DAY
    $walletSessionFile = [System.IO.Path]::GetTempFileName()

    $env:API_BIND_ADDR = "127.0.0.1:18280"
    $env:PUBLIC_BASE_URL = $apiBaseUrl
    $env:MCP_BASE_URL = "http://127.0.0.1:18290"
    $env:PYTHONPATH = Join-Path $repoRoot "crates\sdk-python"
    $env:OPERATOR_API_TOKEN = "agent-bounties-local-sdk-smoke"
    $env:AUTH_PUBLICATIONS_PER_DAY = "4"

    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot "target" }
    $apiPath = Join-Path $targetDir "debug\api.exe"
    if (-not (Test-Path $apiPath)) {
        $apiPath = Join-Path $targetDir "debug\api"
    }
    $api = Start-Process `
        -FilePath $apiPath `
        -WindowStyle Hidden `
        -PassThru `
        -RedirectStandardOutput ".api-sdk-smoke.out.log" `
        -RedirectStandardError ".api-sdk-smoke.err.log"

    try {
        $ready = $false
        for ($i = 0; $i -lt 80; $i++) {
            try {
                $body = Invoke-WebRequest -UseBasicParsing -Uri "$apiBaseUrl/health" -TimeoutSec 2
                if ($body.Content -eq "ok") {
                    $ready = $true
                    break
                }
            }
            catch {
                Start-Sleep -Milliseconds 250
            }
        }
        if (-not $ready) {
            throw "API did not become ready at $apiBaseUrl"
        }

        Invoke-Checked { & $pythonCommand.Source @pythonArgs scripts\seed-smoke-wallet.py --container $env:AGENT_BOUNTIES_TEST_POSTGRES_CONTAINER --output $walletSessionFile }
        $env:AGENT_BOUNTIES_SMOKE_WALLET_SESSION = [System.IO.File]::ReadAllText($walletSessionFile)

        Invoke-Checked { & $pythonCommand.Source @pythonArgs -m agent_bounties.smoke --base-url $apiBaseUrl }
        Invoke-Checked { & $pythonCommand.Source @pythonArgs crates\sdk-python\examples\cofund_claim.py --base-url $apiBaseUrl }

        Push-Location (Join-Path $repoRoot "crates\sdk-typescript")
        Invoke-Checked { npm ci }
        Invoke-Checked { npm run build }
        Invoke-Checked { npm run build:examples }
        Invoke-Checked { node dist/smoke.js --base-url $apiBaseUrl }
        Invoke-Checked { node dist-examples/examples/cofund-claim.js --base-url $apiBaseUrl }
        Pop-Location
    }
    finally {
        if ($api -and -not $api.HasExited) {
            Stop-Process -Id $api.Id -Force
            $api.WaitForExit()
        }
        if ($null -ne $oldApiBind) { $env:API_BIND_ADDR = $oldApiBind } else { Remove-Item Env:API_BIND_ADDR -ErrorAction SilentlyContinue }
        if ($null -ne $oldPublicBase) { $env:PUBLIC_BASE_URL = $oldPublicBase } else { Remove-Item Env:PUBLIC_BASE_URL -ErrorAction SilentlyContinue }
        if ($null -ne $oldMcpBase) { $env:MCP_BASE_URL = $oldMcpBase } else { Remove-Item Env:MCP_BASE_URL -ErrorAction SilentlyContinue }
        if ($null -ne $oldDatabaseUrl) { $env:DATABASE_URL = $oldDatabaseUrl } else { Remove-Item Env:DATABASE_URL -ErrorAction SilentlyContinue }
        if ($null -ne $oldPythonPath) { $env:PYTHONPATH = $oldPythonPath } else { Remove-Item Env:PYTHONPATH -ErrorAction SilentlyContinue }
        if ($null -ne $oldOperatorApiToken) { $env:OPERATOR_API_TOKEN = $oldOperatorApiToken } else { Remove-Item Env:OPERATOR_API_TOKEN -ErrorAction SilentlyContinue }
        if ($null -ne $oldWalletSession) { $env:AGENT_BOUNTIES_SMOKE_WALLET_SESSION = $oldWalletSession } else { Remove-Item Env:AGENT_BOUNTIES_SMOKE_WALLET_SESSION -ErrorAction SilentlyContinue }
        if ($null -ne $oldPublicationLimit) { $env:AUTH_PUBLICATIONS_PER_DAY = $oldPublicationLimit } else { Remove-Item Env:AUTH_PUBLICATIONS_PER_DAY -ErrorAction SilentlyContinue }
        Remove-Item $walletSessionFile -ErrorAction SilentlyContinue
    }
}
finally {
    Pop-Location
}
