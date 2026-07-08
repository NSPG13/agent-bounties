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

function Format-MarkdownList {
    param(
        [object[]] $Items,
        [int] $Limit = 12,
        [string] $Empty = "- None"
    )

    $values = @($Items | Where-Object { $_ } | Select-Object -First $Limit)
    if ($values.Count -eq 0) {
        return $Empty
    }
    return (($values | ForEach-Object { "- $_" }) -join "`n")
}

function Get-DocsContractIssues {
    param([string] $Output)

    if ([string]::IsNullOrWhiteSpace($Output)) {
        return @()
    }
    return @(
        $Output -split "`r?`n" |
            ForEach-Object { $_.Trim() } |
            Where-Object { $_ -match '^[^\s:][^:]+:\d+:' } |
            Select-Object -First 20
    )
}

function New-ConstructiveFeedback {
    param(
        [bool] $DocsOnly,
        [object[]] $RiskyFiles,
        [object[]] $NonDocsFiles,
        [bool] $DocsCheckOk,
        [object[]] $DocsIssues
    )

    $items = @()
    if (-not $DocsOnly) {
        $items += "Split docs-only changes from code or infrastructure changes, or wait for manual maintainer review of the non-doc paths."
    }
    if ($RiskyFiles.Count -gt 0) {
        $items += "Risky paths need line-by-line maintainer review before CI or any upstream collaboration branch is approved."
    }
    if (-not $DocsCheckOk) {
        $items += "Run `cargo run -p cli -- docs-contract-check` locally and update examples to match the current API routes, MCP tools, discovery manifest shape, and request payloads."
    }
    if ($DocsIssues.Count -gt 0) {
        $items += "Start with the first docs-contract issue listed below, then rerun the checker until it reports `docs_contract_check=ok`."
    }
    if ($items.Count -eq 0) {
        $items += "Perform semantic review before approving merge, and keep payment or bounty acceptance separate from code review."
    }
    return $items
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
    Invoke-Checked { git fetch origin "+pull/$Pr/head:$refName" }
    $fetchedOid = (& git rev-parse $refName).Trim()
    if ($global:LASTEXITCODE -ne 0) {
        throw "Unable to resolve fetched review ref: $refName"
    }
    if ($fetchedOid -ne $prJson.headRefOid) {
        throw "Fetched PR head $fetchedOid did not match GitHub head $($prJson.headRefOid); rerun review"
    }
    $global:LASTEXITCODE = 0

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

    $docsIssues = Get-DocsContractIssues $docsCheckOutput
    $collaborationBranchCandidate = $docsOnly -and $riskyFiles.Count -eq 0
    $mainCandidate = $safeForMaintainerCi -and $docsCheckOk
    $recommendedLane = if ($mainCandidate) {
        "main-candidate"
    } elseif ($collaborationBranchCandidate) {
        "collaboration-branch-candidate"
    } else {
        "manual-security-review"
    }
    $feedbackItems = New-ConstructiveFeedback `
        -DocsOnly $docsOnly `
        -RiskyFiles $riskyFiles `
        -NonDocsFiles $nonDocsFiles `
        -DocsCheckOk $docsCheckOk `
        -DocsIssues $docsIssues

    $result = [ordered]@{
        pr = $prJson.number
        title = $prJson.title
        url = $prJson.url
        author = $prJson.author.login
        docs_only = $docsOnly
        safe_for_maintainer_ci = ($safeForMaintainerCi -and $docsCheckOk)
        main_candidate = $mainCandidate
        collaboration_branch_candidate = $collaborationBranchCandidate
        recommended_lane = $recommendedLane
        risky_files = [string[]]@($riskyFiles)
        non_docs_files = [string[]]@($nonDocsFiles)
        docs_contract_check = if ($docsCheckOk) { "ok" } else { "failed" }
        docs_contract_issues = [string[]]@($docsIssues)
        constructive_feedback = [string[]]@($feedbackItems)
        docs_contract_output = $docsCheckOutput
    }
    $result | ConvertTo-Json -Depth 6

    if ($PostReview) {
        if ($mainCandidate) {
            $body = @"
Automated external PR intake passed.

What passed:
- The changed files are docs-only.
- No risky paths were changed.
- `docs-contract-check` passed against the trusted maintainer checkout.

Recommended lane: main-candidate.

Next steps:
- A maintainer should still review the semantics before merging.
- This review does not approve bounty acceptance, payout, or payment settlement.
"@
            Invoke-Checked { gh pr review $Pr --repo $Repo --comment --body $body }
        } else {
            $blockers = @()
            if ($nonDocsFiles.Count -gt 0) {
                $blockers += "Non-doc files changed:`n$(Format-MarkdownList $nonDocsFiles)"
            }
            if ($riskyFiles.Count -gt 0) {
                $blockers += "Risky files changed:`n$(Format-MarkdownList $riskyFiles)"
            }
            if (-not $docsCheckOk) {
                $blockers += "Docs contract check failed:`n$(Format-MarkdownList $docsIssues -Empty '- The checker failed without line-specific issues. Run the command below for full output.')"
            }
            $branchGuidance = if ($collaborationBranchCandidate) {
                "This looks suitable for a collaboration branch such as `collab/pr-$Pr-<short-topic>` if a maintainer wants others to iterate on it without merging to `main` yet. That branch would not imply bounty acceptance or payment approval."
            } else {
                "Do not move this to an upstream collaboration branch automatically. The risky or non-doc paths need manual maintainer security review first."
            }
            $body = @"
Thanks for the contribution. I cannot approve this for `main` yet, but the next repair steps are concrete.

Recommended lane: $recommendedLane.

Why it is blocked:
$(($blockers | ForEach-Object { $_ }) -join "`n`n")

How to fix:
$(Format-MarkdownList $feedbackItems)

Local command to run before pushing an update:

~~~bash
cargo run -p cli -- docs-contract-check
~~~

Collaboration branch guidance:
$branchGuidance
"@
            Invoke-Checked { gh pr review $Pr --repo $Repo --request-changes --body $body }
        }
    }

    if (-not $mainCandidate) {
        exit 1
    }
} finally {
    Pop-Location
}
