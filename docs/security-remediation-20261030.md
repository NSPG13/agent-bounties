# October 30 security and public-source reconciliation

This change imports the hosted runtime source at `2c90e16bce38c8a5925b1af27fbb79a243ff3cc3` into the public repository. The production lineage was maintained in `NSPG13/AB-PRIVATE`; an absent public Git object did not establish an unknown or malicious binary. It did prevent public review. The import includes the actual API/MCP authentication, managed orders, customer reviews, appeals, refund reconciliation, durable authorization and terms-publication implementation. Public website updates are retained.

The change is R3 (authorization, payment evidence and durable history). It does not authorize contract deployment, fund recovery, signer rotation, or changes to payment limits. Existing deployed contracts are immutable.

## Audit disposition and implemented controls

| Critique | Resolution in this change |
| --- | --- |
| CI imports PyYAML before installation | Already fixed by #1717. Keep the public dependency ordering and its later tests. Add production runtime durability checks; minimum Rust is 1.90. |
| Distribution canary metadata | Emit identical acquisition metadata at both `agentizable.ai/acquisition` and the legacy key. Attribution remains analytics, never settlement authority. |
| Unauthenticated funding and unset operator token | Publish the production implementation, which already requires operator authorization for organization-balance contributions and denies absent/blank token configuration. Creator terms publication binds session, wallet, canonical payload and durable idempotency. |
| Unsafe v1 indexing | Bound admission by the RPC safe block and confirmation margin, verify block identity before/after reads, atomically certify cursor/hash, and quarantine a changed prior hash. Bootstrap old cursors by comparing stored events with canonical logs. Never delete events or automatically rewind. Duplicate creation quarantines that bounty rather than hiding unrelated rows. |
| Immutable v1 vulnerabilities | Mark the public manifest recovery-only, remove these mainnet rows from earning readiness, and deny hosted v1 creation/funding/new claims. Preserve historical planning, submissions for existing work, expiry/cancellation and refunds. V2 retains receiver-bound USDC authorization and invalid-proof rejection. |
| Standing-meta liquidity | Remains unsatisfied. Existing liquidity guard reports the failure. Five misleading issue titles were changed to explicitly unavailable. The unrelated Open Competition replenisher cannot satisfy this invariant. No readiness flag or deployment evidence is fabricated. |
| Private/public source drift | Publish the hosted source, compare all runtime source/build inputs and embedded Rust assets against public main, require the comparison in release verification, and observe public revision reachability hourly. A narrow release controller moves the reviewed production inventory to public main and then deploys green public commits. |
| Keeper budget and custody | Registration requires a trusted maintainer association, approved issue label and matching comment sender. Every signing-key workflow requires main in the public repository, an explicit environment, pinned external actions and keys scoped to run steps. New environment policies permit only main. Application release uses only the Render credential. |
| Traceback-controlled no-verdict | Stdout/stderr are bounded evidence hashes, not a verdict classifier. A failing checker exit (1) stays failing even when solver output contains a traceback. Infrastructure exits (2–127), signals and timeouts still fail closed. |
| Auth documentation and invoice refund evidence | Production auth is now published. Incorporate #1671's legacy invoice projection and refund parsers, including full allocation/identity checks and succeeded-only repayment evidence. The legacy invoice HTTP module is retained for review; it is not activated in the production router. Current production uses the separate managed-order payment path. |

## Threat model and rollout

Untrusted contributors can submit code/output, public comments, signatures and malformed requests. They cannot authorize organization-balance debits, choose a verifier's infrastructure classification through output text, spend keeper gas solely by commenting, turn orphaned logs into a paid projection, or obtain a mainnet v1 funding/claim plan from the hosted entry points.

```
public contributor -> signature/session/operator checks -> durable scoped mutation
untrusted solver output -> bounded hashes + checker exit status -> verifier evidence
RPC safe history -> block-hash checkpoint -> canonical projection -> paid language
public green main -> source comparison -> exact Render service/commit -> live verification
```

Before release, run `bash scripts/check.sh`, the portable PostgreSQL release inventory, the real Docker regression sandbox rehearsal and production-compose checks. These use disposable local databases and do not charge customers or sign mainnet transactions. The verifier release remains separately qualified: this patch does not replace an immutable benchmark or silently approve a new signing runtime.

The canonicality migration is additive. During first startup, old history is unavailable until the worker certifies it. A changed block hash leaves a persistent quarantine. Investigate against independent RPC readers and prepare an explicit reviewed reconciliation; restarting the worker does not clear it. No generic self-heal rule may delete event rows, rewind the cursor or clear quarantine. Snapshot database/evidence before any operator reconciliation.

The standalone invoice projection remains tested for historical evidence and should not be mistaken for an enabled checkout service. Do not expose its old HTTP handlers without separately restoring their durable storage integration and reviewing authentication. Existing invoice records and migration files remain intact.

## Remaining immutable/liquidity work

The standing-meta successor must meet `docs/standing-meta-bounty-invariant.md`: reviewed verifier independence design, positive margin, exact child preparation, R4 review, testnet rehearsal, exact bytecode and an action-time funding/signing decision. Existing V2 parents are recovery-reserved; V3 routed parents lack an approved child profile. There is no authorized safe action that can relabel them into compliant live inventory.

Prepare the successor and a bounded funding plan before seeking that final signing decision. Keep the invariant monitor red until canonical funding and verification readiness actually exist. Recovery and new escrow transactions require their own exact evidence; this application release grants no such authority.

## Operational evidence and boundaries

The maintainer requested these security and source-reconciliation changes. Existing
payment permissions and spending limits are preserved. Local canaries use disposable
PostgreSQL and Docker fixtures; the production rollout must pass exact-revision
checks before completion is reported. Retire the legacy application controller and
keep the previous public revision available for a reviewed forward-compatible repair.

The new main-only environments restrict when the existing workflows may run. Existing
repository secrets were not exported, moved or rotated. Environment-scoped secret
migration or signer rotation needs custody-owner execution; a repository administrator
with workflow-write and secret access remains trusted. Full action pins and step-level
key injection reduce third-party action exposure but do not make compromised runners safe.

The current deployed diagnostic patch also remains included: fixed failure classes
separate local request pacing, provider transport and database/head timeouts, retaining
the existing twelve-second freshness deadline and excluding raw error values.
