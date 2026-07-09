param(
    [string]$Repository = "NSPG13/agent-bounties",
    [string]$ApiBaseUrl = "http://127.0.0.1:8080",
    [string]$OperatorToken = $env:OPERATOR_API_TOKEN,
    [int]$Limit = 200,
    [switch]$IncludeOwner
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "GitHub CLI 'gh' is required."
}

$prsJson = gh pr list `
    --repo $Repository `
    --state all `
    --limit $Limit `
    --json number,url,author

$prs = $prsJson | ConvertFrom-Json
$contributors = @{}

foreach ($pr in $prs) {
    if ($null -eq $pr.author -or [string]::IsNullOrWhiteSpace($pr.author.login)) {
        continue
    }
    if ($pr.author.is_bot) {
        continue
    }
    if (-not $IncludeOwner -and $pr.author.login -eq "NSPG13") {
        continue
    }

    $login = $pr.author.login
    if (-not $contributors.ContainsKey($login)) {
        $contributors[$login] = [System.Collections.Generic.List[string]]::new()
    }
    $contributors[$login].Add($pr.url)
}

$headers = @{}
if (-not [string]::IsNullOrWhiteSpace($OperatorToken)) {
    $headers["x-operator-token"] = $OperatorToken
}

$existingByLogin = @{}
try {
    $existingContacts = Invoke-RestMethod `
        -Method Get `
        -Uri "$ApiBaseUrl/v1/contributor-contacts" `
        -Headers $headers
    foreach ($contact in $existingContacts) {
        $existingByLogin[$contact.github_login.ToLowerInvariant()] = $contact
    }
} catch {
    Write-Warning "Could not read existing contributor contacts; import will use public PR history only. $($_.Exception.Message)"
}

foreach ($login in ($contributors.Keys | Sort-Object)) {
    $associatedPrs = $contributors[$login] | Sort-Object -Unique
    $existing = $existingByLogin[$login.ToLowerInvariant()]
    $existingPrs = @()
    if ($null -ne $existing -and $null -ne $existing.associated_prs) {
        $existingPrs = @($existing.associated_prs)
    }
    $mergedPrs = @($associatedPrs + $existingPrs) | Sort-Object -Unique
    $email = $null
    $payoutWallet = $null
    $contactConsent = $false
    $walletConsent = $false
    $outreachAllowed = $false
    $source = "github-pr-history"
    $notes = "Imported from public GitHub PR history. Email and wallet unknown until contributor opt-in."

    if ($null -ne $existing) {
        $email = $existing.email
        $payoutWallet = $existing.payout_wallet
        $contactConsent = [bool]$existing.contact_consent
        $walletConsent = [bool]$existing.wallet_consent
        $outreachAllowed = [bool]$existing.outreach_allowed
        if (-not [string]::IsNullOrWhiteSpace($existing.source)) {
            $source = $existing.source
        }
        if (-not [string]::IsNullOrWhiteSpace($existing.notes)) {
            $notes = $existing.notes
        }
    }

    $body = @{
        github_login = $login
        email = $email
        payout_wallet = $payoutWallet
        associated_prs = @($mergedPrs)
        contact_consent = $contactConsent
        wallet_consent = $walletConsent
        outreach_allowed = $outreachAllowed
        source = $source
        notes = $notes
    } | ConvertTo-Json -Depth 5

    Invoke-RestMethod `
        -Method Post `
        -Uri "$ApiBaseUrl/v1/contributor-contacts" `
        -Headers $headers `
        -ContentType "application/json" `
        -Body $body | Out-Null

    Write-Output "Imported $login with $($mergedPrs.Count) PR(s)."
}

Write-Output "Imported $($contributors.Count) contributor contact record(s)."
