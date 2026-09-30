# Consented Google Ads outcomes (R3)

Status: implementation candidate; production activation is blocked on the gates
below. Owner: Agent Bounties operator. Notice: [#1539](https://github.com/NSPG13/agent-bounties/issues/1539).

## Decision and authority

Reuse signed distribution acquisitions and private account draft recovery; do
not add click IDs to public analytics, agent tools, shared URLs, wallet requests,
terms, or evidence. This does not modify contracts, verifier rules, permissions,
signing, or payout logic. No advertising spend or campaign activation is allowed.

```text
Google click --explicit consent--> private encrypted click + signed acquisition
    --> private saved draft --> login / wallet review --> published terms binding
    --> confirmed Base events --> exclusion / evidence checks --> durable outbox
    --> Google Data Manager --> ingestion diagnostics (NOT campaign credit)
```

Only the wallet owner approves transactions. Only indexed canonical events with
verified block timestamps establish funding or settlement. The API holds an
independent AES-256-GCM key and a separate Google OAuth refresh credential; neither
has wallet authority. The existing acquisition HMAC grants only attribution.
Operator credentials alone authorize cost imports and private reports.

## Privacy and threat model

- Consent is `google-ads-outcomes-v1`, separate from Google Analytics. GPC, DNT,
  analytics opt-out, denied consent and unavailable storage disable collection.
- Click IDs are pseudonymous, not anonymous. Only one bounded gclid/gbraid/wbraid
  is accepted per request. A keyed fingerprint prevents copied-click credit.
- AES-GCM uses a fresh random 96-bit nonce; authenticated data binds the consent
  version. The key must be separate from login, acquisition and wallet secrets.
- Capture removes click query parameters before other page scripts run. The
  initial landing request necessarily reaches hosting infrastructure: configure
  CDN/access-log query redaction before activation, and never publish raw logs.
- At 90 days or withdrawal the worker erases both ciphertext and its keyed
  fingerprint. Minimal campaign, consent and delivery audit metadata remain.
  Offline withdrawal retains only an opaque deletion request and retries.
- Saved draft recovery accepts exactly an opaque acquisition/handoff pair. It is
  excluded from the approval hash and wallet journal, and survives authenticated
  continuation without transmitting click IDs. It conveys no signing authority.
- Requests have a 4-KiB body limit, explicit Origin validation and bounded fields;
  captured clicks are capped at 100 per acquisition. This is not anti-bot identity:
  enforce normal edge rate limits before public enablement. A forged Origin or
  identifier cannot fabricate canonical funding or Google campaign credit.
- The outbox rechecks consent, expiry and exclusions immediately before sending.
  Withdrawal cannot retract a request already in flight or received by Google.
  `accepted_now_ineligible` flags later exclusions/reorgs for operator review and
  provider-side correction; there is no automatic retraction claim.
- Source labels can be self-declared. First-party numbers require a real private
  handoff bound before creation; Google must independently match the ad click.

## Canonical outcomes and retry behavior

Coverage is **autonomous-v1, Base mainnet only**, not the entire platform.
External funding counts only after non-excluded contributions meet the canonical
target and the bounty becomes claimable. Paid additionally requires a confirmed
settlement matching stored evidence hash, solver and round. Creator/solver must
be different wallets. All active wallet exclusion classes and acquisition
canaries are excluded; unknown related wallets cannot be inferred automatically.

One random outbox UUID per `(network, bounty_contract, outcome)` is Google's
`transactionId`, stable across retries. The outcome selects a configured action.
The customer/action destination is pinned before the first send so an ambiguous
retry cannot silently move the same outcome into a different conversion action.
No wallet, email, task, contract address, payment amount or currency is sent.
One leased row per request, five-minute lease fencing, 20-second HTTP timeouts,
exponential backoff capped at one hour. HTTP 401/429/5xx and transport failures
retry; other HTTP errors remain visible as rejected. Google failures never run
inside posting. Validation also transmits identifiers and must be explicitly
enabled; synthetic canaries are never queued, including validation mode.

Poll `requestStatus:retrieve` first after 30 minutes, then hourly while unresolved.
Store only bounded reason enums/counts, not provider payloads. Successful ingestion
is **not** a matched/credited conversion; the report leaves Google credit unknown
until an actual Google Ads report establishes it.

The first acquisition source and first Google campaign remain fixed; later
campaigns are assists. Outcomes after the click's 90-day window remain internally
reportable but cannot upload. No September attribution is guessed or backfilled.

## Migration, failure and containment

Migration 0038 adds private tables/read models and extends distribution rails.
Its rail constraints supersede 0035 in the startup replay path; immutable older
migration files remain untouched. Replay with new-rail rows is tested.
**Do not roll back to a pre-0038 binary after enabling capture:** its startup
constraints do not accept the new rails. Contain by disabling capture/export/
validation on this compatible release; leave purge/withdrawal running. Forward
repair preserves rows and existing canonical history. No destructive down-migration.

Provider credential/secret failures degrade only measurement; the queue remains
durable. Encryption-key loss makes retained clicks unusable, not recoverable from
public activity. Backup access must stay private; restored backups must run the
expiry/withdrawal reconciliation before any export. A key rotation needs a separate
explicit re-encryption plan, not an unreviewed environment replacement.

## Verification / activation gates

1. Unit, Node journey/privacy, Postgres replay/outbox/exclusion, docs and site gates.
2. Stage the exact compatible API/site release while all three flags remain false.
   Preserve the separate private-runtime release pin; do not deploy public main
   over private runtime functionality or change the pin as part of measurement.
3. Approve Google's customer-data attestation; provision the dedicated Data Manager
   OAuth scope and encryption key in the secret manager, never in source/chat.
4. Create the two import actions with Every count, no value, 90-day click window:
   external funded primary, verified paid secondary. Page views remain secondary.
5. Rehearse extension, phone and embedded wallets, login return, top-up, reload,
   remote draft resume, opt-out races, duplicate events, failed funding, missing
   evidence and provider outage. Every controlled journey must pass; known
   consented eligible journey join coverage must be at least 95%. Captured-only
   coverage does not prove that failed capture or unknown visitors were measured.
6. An operator-classified wallet may fund a 2-USDC canary plus exact verifier fee
   only after the person's wallet approval. Canary acquisition remains excluded
   from customer metrics and all production Google requests. Validate real Google
   requests only with a genuine consented eligible click, never fabricated IDs.
7. Confirm actual Google campaign credit on the first genuine eligible customer
   before scaling. Existing campaigns remain stopped pending separate spend approval.

Pending at implementation time: Google's attestation/actions and credentials,
staging/live browser wallet matrix, production migration/deployment, CDN log policy,
approved mainnet canary and first genuine Google-credited customer. Local tests
are not evidence that these gates passed.
