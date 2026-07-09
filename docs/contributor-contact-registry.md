# Contributor Contact And Thank-You Registry

Use the contributor contact registry when maintainers need to track historic
contributors, thank-you USDC distributions, payout-wallet opt-ins, and private
outreach consent.

The registry is operator-only. Do not expose it as a public signup form until
the deployment has a real consent screen, retention policy, export path, and
deletion path.

## Public Outreach Rules

- Public GitHub comments may ask contributors to share a Base-compatible wallet
  address if they want to opt in to a thank-you USDC distribution.
- Public GitHub comments must not ask contributors to post email addresses.
- Email can be stored only when the contributor provides it through a private or
  authenticated channel and explicitly consents to outreach.
- Wallet addresses can be stored only when the contributor explicitly opts in to
  receiving payout or thank-you funds at that address.
- A thank-you distribution is not bounty acceptance, merge approval, escrow
  release, or settlement evidence.
- Payment state changes still require deterministic verifier output, operator
  decision, and reconciled Stripe/Base evidence.

## Suggested Public Comment

```text
Thank you for participating in Agent Bounties. We are preparing a small
thank-you USDC distribution today for historic external contributors.

If you want to opt in and have not already shared one, please reply with a
Base-compatible wallet address. Do not post an email address publicly. If you
want private email/contact updates, a maintainer can arrange a private opt-in
path.

This thank-you is separate from bounty acceptance, PR merge approval, payout
approval, or escrow settlement. Any actual transfer will be recorded only after
on-chain evidence is available.
```

## API

`POST /v1/contributor-contacts` creates or updates a contributor record by
case-insensitive GitHub login. `GET /v1/contributor-contacts` lists stored
records. Both routes require `OPERATOR_API_TOKEN` when hosted operator auth is
configured.

Import historic public PR participants from GitHub after the API is running:

```powershell
$env:OPERATOR_API_TOKEN = "<operator-token>"
.\scripts\import-github-contributors.ps1 `
  -Repository "NSPG13/agent-bounties" `
  -ApiBaseUrl "https://api.example.com"
```

The import stores GitHub login and associated PR URLs only. `email` and
`payout_wallet` remain `null` until the contributor opts in. If a record already
exists, the import reads it first and preserves existing consent, email, wallet,
source, and notes fields while merging PR URLs.

Example wallet-only opt-in:

```powershell
$body = @{
  github_login = "qilu13"
  email = $null
  payout_wallet = "0x1111111111111111111111111111111111111111"
  associated_prs = @("#24", "#43", "#59")
  contact_consent = $false
  wallet_consent = $true
  outreach_allowed = $false
  source = "github-comment-opt-in"
  notes = "Contributor supplied a Base-compatible wallet in a public PR or issue comment."
} | ConvertTo-Json

Invoke-RestMethod `
  -Method Post `
  -Uri "http://127.0.0.1:8080/v1/contributor-contacts" `
  -Headers @{ "x-operator-token" = $env:OPERATOR_API_TOKEN } `
  -ContentType "application/json" `
  -Body $body
```

Example private outreach opt-in:

```json
{
  "github_login": "example-contributor",
  "email": "contributor@example.com",
  "payout_wallet": null,
  "associated_prs": ["#123"],
  "contact_consent": true,
  "wallet_consent": false,
  "outreach_allowed": true,
  "source": "private-opt-in",
  "notes": "Contributor gave email privately and opted into project updates."
}
```

## Stored Fields

- `github_login`: public GitHub username.
- `email`: optional private email, stored only with `contact_consent=true`.
- `payout_wallet`: optional Base-compatible payout wallet, stored only with
  `wallet_consent=true`.
- `associated_prs`: PR numbers or URLs tied to the contributor's work.
- `contact_consent`: whether private contact data may be stored.
- `wallet_consent`: whether the wallet may be used for payout or thank-you
  distribution planning.
- `outreach_allowed`: whether project outreach is allowed; requires
  `contact_consent=true`.
- `source`: where the opt-in came from.
- `notes`: maintainer-only context. Do not put secrets or raw payment data here.

Unknown emails should be stored as `null`. Do not scrape or infer emails from
Git commits, profiles, or third-party enrichment services for outreach.
