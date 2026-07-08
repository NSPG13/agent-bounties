param(
    [Parameter(Mandatory = $true)]
    [int] $Pr,
    [string] $Repo = "NSPG13/agent-bounties",
    [switch] $PostReview
)

$ErrorActionPreference = "Stop"
if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = Join-Path $repoRoot "target\pr-review"
New-Item -ItemType Directory -Force -Path $targetRoot | Out-Null

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

function Test-DocsPath {
    param([string] $Path)
    return (
        $Path -eq "README.md" -or
        $Path -eq "AGENTS.md" -or
        $Path -eq "llms.txt" -or
        $Path.StartsWith("docs/") -or
        $Path.StartsWith("examples/") -or
        $Path.StartsWith(".github/ISSUE_TEMPLATE/")
    )
}

function Test-RiskyPath {
    param([string] $Path)
    return (
        $Path.StartsWith(".github/workflows/") -or
        $Path.StartsWith("scripts/") -or
        $Path.StartsWith("contracts/") -or
        $Path.StartsWith("migrations/") -or
        $Path.StartsWith("crates/") -or
        $Path -eq "Cargo.toml" -or
        $Path -eq "Cargo.lock" -or
        $Path.EndsWith("package.json") -or
        $Path.EndsWith("package-lock.json")
    )
}

Push-Location $repoRoot
try {
    $prJson = gh pr view $Pr --repo $Repo --json number,title,url,author,headRefOid,files | ConvertFrom-Json
    $changedFiles = @($prJson.files | ForEach-Object { $_.path })
    if ($changedFiles.Count -eq 0) {
        throw "PR #$Pr has no changed files"
    }

    $riskyFiles = @($changedFiles | Where-Object { Test-RiskyPath $_ })
    $nonDocsFiles = @($changedFiles | Where-Object { -not (Test-DocsPath $_) })
    $docsOnly = $nonDocsFiles.Count -eq 0
    $safeForMaintainerCi = $docsOnly -and $riskyFiles.Count -eq 0

    $refName = "refs/remotes/origin/pr-$Pr-review"
    Invoke-Checked { git fetch origin "pull/$Pr/head:$refName" }

    $worktreePath = Join-Path $targetRoot "pr-$Pr"
    $targetRootFull = [System.IO.Path]::GetFullPath($targetRoot)
    $worktreeFull = [System.IO.Path]::GetFullPath($worktreePath)
    if (-not $worktreeFull.StartsWith($targetRootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to use unsafe worktree path: $worktreeFull"
    }
    if (Test-Path $worktreeFull) {
        Invoke-Checked { git worktree remove --force $worktreeFull }
    }
    Invoke-Checked { git worktree add --detach $worktreeFull $refName }

    $docsCheckOk = $false
    $docsCheckOutput = ""
    try {
        $docsCheckStdout = Join-Path $targetRoot "pr-$Pr-docs-contract-check.stdout.log"
        $docsCheckStderr = Join-Path $targetRoot "pr-$Pr-docs-contract-check.stderr.log"
        foreach ($path in @($docsCheckStdout, $docsCheckStderr)) {
            if (Test-Path $path) {
                Remove-Item -LiteralPath $path -Force
            }
        }
        $process = Start-Process `
            -FilePath "cargo" `
            -ArgumentList @("run", "-p", "cli", "--", "docs-contract-check", "--root", $worktreeFull, "--contract-root", $repoRoot) `
            -RedirectStandardOutput $docsCheckStdout `
            -RedirectStandardError $docsCheckStderr `
            -Wait `
            -PassThru `
            -WindowStyle Hidden
        $docsCheckExitCode = $process.ExitCode
        $stdoutText = if (Test-Path $docsCheckStdout) { Get-Content -Raw $docsCheckStdout } else { "" }
        $stderrText = if (Test-Path $docsCheckStderr) { Get-Content -Raw $docsCheckStderr } else { "" }
        $docsCheckOutput = (@($stdoutText, $stderrText) -join "`n").Trim()
        if ($docsCheckExitCode -eq 0) {
            $docsCheckOk = $true
        }
    } finally {
        Invoke-Checked { git worktree remove --force $worktreeFull }
    }

    $result = [ordered]@{
        pr = $prJson.number
        title = $prJson.title
        url = $prJson.url
        author = $prJson.author.login
        docs_only = $docsOnly
        safe_for_maintainer_ci = ($safeForMaintainerCi -and $docsCheckOk)
        risky_files = $riskyFiles
        non_docs_files = $nonDocsFiles
        docs_contract_check = if ($docsCheckOk) { "ok" } else { "failed" }
        docs_contract_output = $docsCheckOutput
    }
    $result | ConvertTo-Json -Depth 6

    if ($PostReview) {
        if ($safeForMaintainerCi -and $docsCheckOk) {
            $body = "Automated external PR intake passed static docs-only review and docs-contract-check. This does not approve merge or payment; a maintainer still needs to review semantics and decide whether to approve CI."
            Invoke-Checked { gh pr review $Pr --repo $Repo --comment --body $body }
        } else {
            $body = "Automated external PR intake failed. risky_files=$($riskyFiles -join ', ') non_docs_files=$($nonDocsFiles -join ', ') docs_contract_check=$($result.docs_contract_check). Maintainer review required; do not approve CI yet."
            Invoke-Checked { gh pr review $Pr --repo $Repo --request-changes --body $body }
        }
    }

    if (-not ($safeForMaintainerCi -and $docsCheckOk)) {
        exit 1
    }
} finally {
    Pop-Location
}
