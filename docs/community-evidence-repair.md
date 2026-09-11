# Community evidence repair

Maintainer notice: [#1417](https://github.com/NSPG13/agent-bounties/issues/1417).

## A funded bounty can still be unavailable

Funding, an executable verifier, a solver's work and canonical settlement are
separate facts. Read the current opportunity's `next_action.instructions` and
its canonical feed's `verification_readiness_reason` before starting work.
The board's **Direct tasks** filter excludes competitions and standing child
funding programs. Direct tasks can still require a refundable bond, gas and
execution costs. An empty direct-task view is not an invitation to spend money
to become eligible for a competition.

For Safari issue #927, the observed blocker is an unapproved benchmark source
and digest. Funding it again cannot repair that. The immutable benchmark and
both committed verifier paths need review and a successful rehearsal before
its readiness can change. A UI label, pull request, fixture or maintainer
comment cannot approve the benchmark or replace the committed policy.

A maintainer negative control against the exact pinned source and container
accepted a 33-byte PNG header with no image data and invented browser metadata.
This demonstrates a validation gap, not a Safari capture. Keep this benchmark
unapproved. A replacement must decode the full image, reject malformed input,
and state whether browser/run provenance is independently verified or merely
reported. Review and rehearse replacement terms before offering new work;
already-funded immutable terms must not silently inherit a different checker.

## Child-bounty submissions

This applies to #1354–#1357, #1408 and #1410–#1412, and to similar submissions.
Preserve the useful specification and benchmark as a **draft** until the exact
published terms, deployed child contract and canonical funding exist.

Use this review block in the child specification:

```text
State: draft — not a live paid opportunity
Parent contract: <exact contract from immutable parent terms>
Child contract: not deployed
Child terms hash: not published
Child creation/funding evidence: not available
Verifier source and digest approval: pending
Different solver's canonical settlement: not available
Parent acceptance and payment: not verified
```

Replace a pending value only with independently checked evidence. Retain the
source repository, exact source commit, subdirectory, runner command and
benchmark digest together. A digest alone does not pin executable code.

For an eligible routed V3 parent, the existing MCP tool
`prepare_standing_meta_v2_child` prepares child terms and ordered wallet calls.
The legacy tool name is retained for compatibility. It does not activate a
recovery-reserved V2 parent. Inspect current readiness before calling it; review
and authorize the exact funding commitment through the normal wallet flow.
After a different solver completes the child, reconcile its canonical
`BountySettled` event before submitting parent acceptance evidence. A green
benchmark on local fixtures establishes none of those live facts.

## GMV submissions

This applies to #1390–#1398, #1400–#1403 and #1406–#1407. Preserve the campaign's
committed policy and use one structural validator instead of separate copies
that report `ready: true` or `status: eligible` for caller-provided JSON:

```sh
node scripts/gmv-manifest-preflight.mjs manifest.json policy.json
node --test scripts/test-gmv-manifest-preflight.mjs
```

The policy input has `competition`, `bounty_id`, `epoch_id`,
`verification_policy_hash`, integer `window_start`/`window_end`, and explicit
`excluded_wallets`/`excluded_bounty_contracts` lists. The manifest retains the
existing GMV attribution schema and fields, and adds a required `entrant`.
Its `settlement` (or `settlements` array) contains creator, solver, funder,
child contract, integer settlement time, transaction hash, integer log index,
and **decimal-string** `gmv_base_units`. Never coerce money through JavaScript
`Number`. The entrant must match the funding wallet and differ from the solver.

Exit 0 means structurally valid input only. The output always has
`ready: false`, `eligible: false`, and `status: settlement_unverified`.
Neither the manifest nor the caller-selected policy is authenticated here.
Update positive benchmark expectations to `structure_valid`, retaining those
false eligibility fields. Require independent canonical provenance, immutable
policy binding, exact entrant attribution, current exclusions and the
committed snapshot quorum before making an eligibility decision. Use the
existing canonical GMV snapshot/proof flow; never promote a local fixture into
its attested snapshot.

## Direct evidence and x402 phases

The risk crate's direct evidence checklist preserves the contribution from
[#948](https://github.com/NSPG13/agent-bounties/pull/948), including the author's
artifact traversal repair. Its additional route checks reject normalization
tricks in repository, pull-request and check-run URLs. Structural validity does
not verify a download, its digest, a successful CI run or a settlement.

Run the reusable synthetic asynchronous phase vectors with:

```sh
python3 examples/x402-async-phases/verify.py
```

The [fixture README](../examples/x402-async-phases/README.md) specifies the
restricted canonical JSON domain, eight outcomes and adapter trust boundary.
These are application test vectors for the x402 discussions, not an adopted
extension or real chain receipts. Funding, verified delivery, outcome
settlement and completeness must remain independently assessed.

## Reviewed source fingerprint

The Rust changes in #1418 change the worker build fingerprint from
`25f338acf74ba2a612cccdf5dc4e167ac3d0e929f2d0ad1d6850ca701f172821` to
`efda90427f4a1c0a99105789ac1aef6bab0152237378902f65768908636bf228`.
The previous value was reproduced from an isolated archive of main. The four
changed Rust files contain read-only projections, an advisory checklist and
tests; they do not change the signing pipeline or committed verifier policy.
The three workflow pins and current watchdog rehearsal/checker fingerprints
are refreshed together after review. Previously committed benchmark tuples
remain pinned to their original source.

The signing runtime remains
`469bf155b1bbc5f19ee91ee41172e113cd5baea6f9d1f2d574d88672b1999ddc`.
The pipeline's 28 tests, source guard's 12 tests and the worker/verifier SDK's
64 non-ignored tests pass. Neither a fingerprint refresh nor an offline
rehearsal authorizes new signing identities, benchmark approval or payment.
