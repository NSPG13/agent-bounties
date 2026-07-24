# Self-Healing Operations

Self-healing means bounded recovery from observable failure. It does not mean
that an AI or background process can change money, authority, contracts, or
canonical history until the application appears healthy.

The system uses four safeguards:

1. **Prevention:** typed state machines, immutable contracts, idempotency,
   property tests, abuse fixtures, and exact deployment attestations.
2. **Detection:** health probes, revision headers, indexer heartbeats, cursor
   lag, verifier readiness, webhook backlog, SLOs, and canonical invariants.
3. **Recovery:** allowlisted R0-R2 actions with bounded attempts and explicit
   postconditions.
4. **Learning:** every incident becomes a deterministic fixture before a repair
   is accepted.

The machine policy is
[`ops/self-healing-policy.json`](../ops/self-healing-policy.json). The policy is
executable documentation: CI rejects malformed policy, prohibited automatic
actions, and recovery-corpus regressions.

## Current Recovery Topology

```text
GitHub scheduled probe ----> API /health + revision ----+
                         +-> MCP /health + revision ----+-> recovery plan
                         |                               |   + artifact/alert
Render health supervisor +-> restart unhealthy web <---+

Base indexer -> poll -> persist heartbeat/events/cursor
                  | typed RPC/SQL transport error
                  v
             5s,10s,20s...120s retry from persisted cursor
                  | eight consecutive failures
                  v
             exit -> Render replaces worker process

malformed log/cursor/config/integrity failure -> failed heartbeat -> halt ingestion
                                                    +-> contain + incident

successful main CI -> exact-SHA Render deploy controller -> API/MCP/worker live
                                                       +-> exact web revision proof
```

The deploy controller is separate from the public observer. It is allowed to
deploy the latest successful reviewed application revision reachable from
main. It resolves services against the canonical repository and branch,
disables native commit-trigger deploys, polls all three Render deploy records,
and writes redacted evidence. A newer failed main commit cannot suppress the
last known-good release, and an older successful run skips after a newer
successful run exists. It cannot deploy an unrelated branch, contract, wallet,
or payment action. A missing credential, ambiguous service, failed build,
timeout, or health revision mismatch fails the workflow and leaves the
read-only operational control loop in its existing fail-closed state.

Render probes web services every few seconds, stops routing after sustained
failure, and automatically restarts an instance after 60 seconds of failed
health checks. See [Render health checks](https://render.com/docs/health-checks).
The root Blueprint configures `/health` for API and MCP. `GET /health` remains a
cheap liveness contract: `200`, body `ok`, protocol header, and exact revision
header. Dependency freshness belongs on separate readiness surfaces so an
external dependency outage does not create a restart storm.

The Base worker now recovers only typed RPC transport/status/provider and SQL
transport errors with capped exponential backoff. Every failed attempt writes
the existing failure heartbeat when possible. A retry starts from the persisted
monotonic cursor; event upserts remain idempotent. Eight consecutive failures
exhaust the local budget and exit so the platform supervisor can replace the
process. Malformed responses, invalid emitters, decode/config/cursor errors, and
other unclassified failures halt ingestion after the failed heartbeat; they are
not retried or converted into canonical evidence.

## SLOs And Error Budgets

Initial public-beta objectives:

| Signal | Objective | Window | Failure response |
| --- | --- | --- | --- |
| API and MCP `/health` | 99.5% success | rolling 30 days | retry, platform restart, then incident |
| API/MCP revision agreement | 100% | every release probe | block rollout |
| Indexer successful/skipped heartbeat age | <= 90 seconds | continuous | resume from cursor, then replace process |
| Indexer confirmed cursor lag | <= 20 Base blocks | continuous | resume or attest RPC failover |
| Claimable bounty with `verification_ready=false` | 0 | every feed build | suppress from earning inventory |
| Valid Stripe webhook reconciliation | 99% within 5 minutes | rolling 30 days | idempotent replay only with all bindings |
| False funded/paid claim | 0 | lifetime | SEV0 containment |
| Contract/runtime hash mismatch | 0 | every planner and release probe | SEV0 containment |
| Chain read-model recovery | RPO 0, RTO <= 1 hour | incident | rebuild from confirmed logs |
| Off-chain terms/evidence/audience data | RPO <= 24 hours, RTO <= 4 hours | incident | database restore with explicit approval |

Correctness SLOs have no error budget. One false payment claim, ledger
conservation failure, cursor regression, or runtime-hash mismatch freezes
value-changing hosted features until reconciled. Availability consumes a
budget: if more than half of the monthly availability budget is spent in seven
days, feature releases pause until the cause is fixed.

## Remediation Matrix

| Failure | Automatic action | Required evidence | Escalation |
| --- | --- | --- | --- |
| one or two API/MCP probe failures | bounded read-only retry | no side effect | after probe budget |
| three API/MCP failures | Render restarts same deployed service | no integrity/revision mismatch | if post-restart smoke fails |
| typed RPC/SQL transport error | retry with capped backoff | persisted monotonic cursor | after eight failures |
| malformed log, emitter, cursor, config, or unclassified indexer error | halt ingestion | failed redacted heartbeat | reviewed correction or redeploy |
| stale indexer heartbeat/lag | resume from persisted cursor | canonical event integrity and idempotent upsert | any cursor or event mismatch |
| primary RPC unavailable | switch only to preconfigured attested RPC | chain id, safe block, factory and implementation hashes | no attested endpoint |
| verifier service unavailable | remove affected bounty from earning feed | read-model-only change | verifier policy or contract mismatch |
| delayed Stripe webhook | replay one verified event | signature, event id, amount and destination binding | any missing binding |
| reviewed main revision not deployed | deploy latest successful-CI application SHA once | reachable main commit, canonical service binding, terminal `live`, exact web health revision | failed build, timeout, service ambiguity, or revision mismatch |
| low claimable inventory | publish alert and creation plan | canonical inventory count | wallet funding always needs authority |
| database unavailable/corrupt | none | backup and migration evidence | restore or failover approval |
| payment/ledger/hash mismatch | none | independent canonical reconciliation | SEV0 incident |

`switch_to_attested_rpc` and `replay_verified_webhook` are policy-allowlisted,
but their runtime adapters must remain disabled until their required telemetry
is available and their own failure fixtures pass. Policy eligibility is not
proof that an adapter is deployed.

`deploy_reviewed_application_revision` is implemented by the dedicated GitHub
Actions workflow, not by the public observer. Its only secret is the GitHub
Actions `RENDER_API_KEY`; application containers and scheduled probes do not
receive it. Provisioning or rotating that credential remains an explicit R3
access change; bounded use for an already-reviewed application SHA is R2.

## Prohibited Automatic Repair

The controller has no authority to:

- sign or approve wallet transactions;
- fund a bounty or move USDC;
- submit a verifier verdict or settle/refund a bounty;
- deploy, pause, upgrade, or replace a contract;
- rotate secrets, keys, roles, rulesets, or access controls;
- restore a database, run a destructive migration, or delete records;
- edit terms, evidence, ledger entries, payouts, canonical events, or chain
  cursors;
- call a non-attested RPC as authoritative;
- describe funding or payment from a plan, webhook receipt, transaction hash,
  AI output, or hosted row.

Those are R3-R4 operations and require the authority committed by the protocol
or an explicit maintainer incident decision.

## Controller Commands

Validate the machine policy:

```powershell
python scripts/self_heal.py validate-policy `
  --policy ops/self-healing-policy.json
```

Run RecoveryBench:

```powershell
python scripts/self_heal.py bench `
  --policy ops/self-healing-policy.json `
  --fixtures ops/fixtures/recovery-cases.json `
  --output target/operations/recovery-bench.json
```

Evaluate a trusted runtime snapshot:

```powershell
python scripts/self_heal.py evaluate `
  --policy ops/self-healing-policy.json `
  --snapshot path/to/snapshot.json `
  --output target/operations/recovery-plan.json
```

Run read-only public probes:

```powershell
python scripts/self_heal.py observe `
  --policy ops/self-healing-policy.json `
  --api-url https://api.agentbounties.app `
  --mcp-url https://mcp.agentbounties.app `
  --expected-revision <40-character-git-sha> `
  --snapshot-out target/operations/snapshot.json `
  --plan-out target/operations/recovery-plan.json
```

The scheduled `Operational Control Loop` workflow runs the policy/corpus gate,
probes production, uploads the snapshot and plan for 30 days, and fails closed
when containment or manual escalation is required. The public probe does not
hold credentials and cannot mutate the platform.

## Trusted Runtime Snapshot

`agent-bounties/operations-snapshot-v1` accepts component observations for:

- API and MCP state, failures, protocol, and revision;
- durable database state/freshness;
- indexer heartbeat age, lag, and cursor monotonicity;
- Base RPC state and attested failover availability;
- verifier readiness and incorrectly advertised inventory;
- Stripe webhook backlog plus signature/idempotency/binding evidence;
- claimable inventory count;
- canonical event, contract hash, cursor, ledger, and payment-evidence
  invariants.

Public HTTP probes can assert availability and revision only. Unknown internal
invariants remain `null`; they are never silently assumed true. Automatic
actions that require canonical integrity are therefore available only to a
trusted runtime observer with those facts.

## Incident Levels

- **SEV0:** suspected loss, unauthorized value movement, false paid/funded
  claim, contract/hash mismatch, ledger failure, key exposure, or canonical
  event/cursor corruption. Contain immediately; no automatic repair.
- **SEV1:** all earning/funding/verification paths unavailable, database
  unavailable, or widespread incorrect claimability. Incident owner within 15
  minutes.
- **SEV2:** one service degraded, index lag beyond SLO, verifier subset down,
  or webhook delay with intact evidence. Automatic recovery may proceed.
- **SEV3:** non-critical distribution, analytics, template, or documentation
  degradation.

Never put secrets, private evidence, personal data, or unredacted provider logs
in a public incident issue. Record public facts and keep sensitive evidence in
the approved private system.

## Recovery Verification

A repair is successful only when:

1. the triggering signal returns within SLO;
2. API and MCP advertise the same expected revision;
3. the indexer cursor advances monotonically from the stored cursor;
4. canonical event and contract-hash checks pass;
5. production smoke passes;
6. the incident fixture passes and all previous RecoveryBench cases still pass.

A process restart, successful HTTP request, transaction receipt, or cleared
alert is not enough by itself.

## RecoveryLoop

For each incident:

1. capture the smallest redacted snapshot that reproduces the failure;
2. add it to `ops/fixtures/recovery-cases.json` with the required action and
   forbidden actions;
3. prove the fixture fails under the old policy or code;
4. change one policy, detector, or repair candidate;
5. run unit, RecoveryBench, service, Postgres, and payment/contract gates by
   risk class;
6. retain the candidate only if the incident case improves and no prior case
   regresses;
7. deploy a bounded canary and verify SLO recovery.

AI may summarize logs, cluster incidents, propose hypotheses, or rank repair
candidates. It cannot fabricate missing telemetry, waive a prerequisite, or
authorize an R3-R4 action.
