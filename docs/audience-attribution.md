# Audience Attribution And Outreach Audit

The audience registry answers four operational questions without turning public
participation into a private-data collection exercise:

1. Who has interacted with the network?
2. Which public actions did they take, and did they return?
3. Who was asked how they found the project, and who answered?
4. Which sources produced real bounty posts, reconciled funding, solving,
   payouts, stars, upvotes, or shares?

The registry is operator-only. It complements, but does not replace,
`contributor_contacts`:

- `audience_members` stores a public provider identity, public profile URL,
  roles, and first/last-seen timestamps.
- `audience_interactions` stores idempotent public events and attribution links.
- `discovery_responses` stores structured answers only when they have a public
  source URL or explicit private-storage consent.
- `outreach_attempts` stores one prompt event per participant. Private email
  outreach is rejected unless the contributor contact has both contact consent
  and outreach permission.
- `contributor_contacts` remains the only place for opt-in email and payout
  wallet data.

Do not scrape or infer email addresses, wallets, private messages, or account
ownership. Do not copy raw GitHub comment bodies into the registry.

Keep only the fields needed for attribution and outreach coverage. On a
verified deletion request, an operator can delete the corresponding
`audience_members` row; interactions, discovery responses, and outreach rows
are removed by foreign-key cascade. There is intentionally no unauthenticated
public deletion endpoint.

## GitHub Audit

Generate a read-only live audit:

```powershell
python scripts\github_audience_audit.py `
  --repository NSPG13/agent-bounties `
  --output target\github-audience-audit.json
```

The command indexes public issue, PR, comment, review, bounty reaction, and
stargazer evidence. A `/agent-bounty fund` comment is recorded as
`funding_signaled`, never `bounty_funded`; only reconciled payment evidence may
create the latter. The same rule applies to claim signals versus accepted
claims.

Natural-language answers are listed under `discovery_answer_candidates` with a
public source URL. They are not auto-interpreted. A maintainer must read the
source, summarize the answer faithfully, and use
`POST /v1/audience/discovery-responses`. A reviewed JSON array can also be
passed with `--curated-responses path/to/responses.json`; the importer accepts
only public source URLs and never turns that file into private-contact consent.

After reviewing the dry-run output, sync public identity, interaction, and
outreach records to a running API:

```powershell
$env:OPERATOR_API_TOKEN = "<operator-token>"
python scripts\github_audience_audit.py `
  --repository NSPG13/agent-bounties `
  --output target\github-audience-audit.json `
  --sync `
  --api-base-url https://api.example.com `
  --curated-responses scripts\fixtures\github_discovery_responses.curated.json
```

The import is idempotent by provider identity and provider event ID. Replaying
the same audit does not inflate participants, interactions, stars, or outreach
counts.

## Operator API

- `POST|GET /v1/audience/members`
- `POST|GET /v1/audience/interactions`
- `POST|GET /v1/audience/discovery-responses`
- `POST|GET /v1/audience/outreach-attempts`
- `GET /v1/audience/report`

The report exposes not-asked-or-answered handles, asked-without-response
handles, repeat participation, and attributed conversion counts. It is an operational report,
not payment evidence and not a basis for accepting work.

## GitHub Stars

An AI agent can star a repository only while acting as an authenticated GitHub
user with write access to that user's starring permission. The action must be
explicitly authorized by the human or account owner. Never auto-star, trade
payment for a star, or treat a star as proof of work, funding, or settlement.
