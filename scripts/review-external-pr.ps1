param(
    [Parameter(Mandatory = $true)][int] $Pr,
    [string] $Repo = "NSPG13/agent-bounties",
    [string] $CollaborationBranch,
    [switch] $CreateCollaborationBranch,
    [switch] $PostReview
)

$ErrorActionPreference = "Stop"
$python = Get-Command python -ErrorAction SilentlyContinue
$pythonArgs = @()
if (-not $python) {
    $python = Get-Command py -ErrorAction SilentlyContinue
    $pythonArgs = @("-3")
}
if (-not $python) { throw "python or py is required for external PR review" }
$reviewArgs = @((Join-Path $PSScriptRoot "review_external_pr.py"), "--pr", $Pr, "--repo", $Repo)
if ($CollaborationBranch) { $reviewArgs += @("--collaboration-branch", $CollaborationBranch) }
if ($CreateCollaborationBranch) { $reviewArgs += "--create-collaboration-branch" }
if ($PostReview) { $reviewArgs += "--post-review" }
& $python.Source @pythonArgs @reviewArgs
exit $LASTEXITCODE
