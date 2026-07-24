# Direct competitor intelligence

This capability maintains a daily, evidence-bound view of direct competitors:
platforms where a poster can offer a reward for work and another participant can
claim or submit that work. It deliberately excludes grant allocators, wallets,
agent infrastructure, generic job boards, bounty aggregators, and specialist
security-only programs.

The reviewed registry is [`ops/competitors/direct-competitors.json`](../ops/competitors/direct-competitors.json). Each entry must provide a plain-language
inclusion reason, first-party inclusion evidence, canonical URL, social and
repository links where known, public capabilities, and only reviewed HTTPS
sources. The initial registry covers Agent Bounty, Algora, Opire, Bountycaster,
Pump Go.fun, and ClawTasks. It is a monitored registry, not an assertion that
the six names exhaust every possible market; a candidate is added only after it
passes the inclusion rule with public evidence.

## What is stored

The additive `0012_competitor_intelligence.sql` migration stores:

- current brand, URL, direct-market rationale, status, social/repository links,
  and evidenced functionality;
- append-only daily runs, source observations, source-content hashes, extracted
  public metrics, and detected changes; and
- a machine-readable JSON report and human-readable Markdown report for each
  run.

Sources are public and fetched read-only. The database keeps URLs, timestamps,
HTTP status, a SHA-256 content fingerprint, bounded extracted metrics, and a
bounded failure reason. It does **not** retain page copies, cookies, login data,
API tokens, private profiles, user submissions, or unaudited inferred metrics.

Platform-reported adoption figures remain platform-reported. A source-content
change only proves that the retrieved public source changed; it does not prove
the reason for a change, independent adoption, funding, or payment.

## Daily operation

The `Direct competitor intelligence` workflow runs daily at 07:22 UTC and can
also be started manually. It installs only the Postgres driver, executes:

```bash
python scripts/competitor_intelligence.py \
  --database-url "$COMPETITOR_INTELLIGENCE_DATABASE_URL" \
  --require-database \
  --json-out target/competitor-intelligence/daily-report.json \
  --markdown-out target/competitor-intelligence/daily-report.md
```

and publishes the Markdown report to the Actions run summary plus a 90-day
artifact. GitHub notification settings can notify maintainers about a failed
scheduled run. The workflow requires the repository Actions secret
`COMPETITOR_INTELLIGENCE_DATABASE_URL`, restricted to the production Postgres
database. A missing secret fails loudly rather than generating a report that
claims a database update happened.

The collector persists the run before it begins, makes one bounded request per
configured source, compares each observation to the previous completed run, and
then stores the report. A source failure is recorded as `source_failed` and
never deletes a prior observation. A recovery is recorded as `source_recovered`.
The first successful observation is a baseline and intentionally produces no
invented change.

## Review and recovery

This is an R3 durable-data change because it adds persistent external-business
observations. Its authority is limited to the reviewed registry and database
write access. It has no wallet, payment, deployment, credential-rotation,
competitor-account, or automated product-change authority.

To add a competitor, first establish it meets the inclusion rule from a
first-party public source, then update the registry and its tests. To retire a
competitor, change its status to `inactive`; do not delete its history. If a
source becomes unreliable, remove or replace that configured source and retain
the historical failure evidence. A bad extraction rule should be fixed forward
with a new observation; do not rewrite prior reports.

Focused checks:

```powershell
python scripts/test_competitor_intelligence.py -v
python scripts/check-migration-history.py
cargo test -p db competitor_intelligence_migration_is_evidence_bound_and_additive
```
