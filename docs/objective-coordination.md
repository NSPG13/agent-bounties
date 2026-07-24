# Objective And Contribution Coordination

Objective v1 coordinates several monetary and non-monetary contributions toward
one desired outcome. It does not replace the autonomous bounty protocol. A
canonical bounty remains the paid execution and settlement primitive; the
objective is the off-chain, signed coordination aggregate that links those
primitives.

```text
requesting party creates objective
  -> providers propose exact value bundles
  -> declared authority accepts one immutable bundle
  -> contributors offer -> are selected -> submit -> are verified
  -> all mandatory needs and funding become ready
  -> provider submits final outcome
  -> final policy verifies it
  -> matching canonical settlement proves any payment
  -> objective completes
```

Do not skip states. An offer is not selection, a submission is not verification,
verification is not payment, and supporting work is not the final outcome.

## Roles And Authority

Every participant has an EVM wallet and one explicit participant kind. The
objective separately declares:

- the requesting party;
- beneficiaries;
- affected parties and the expected effect on each;
- the provider;
- contributors and verifiers;
- the objective authority, its members, threshold, and public explanation.

Roles may overlap, but overlap is never inferred. Funding does not grant
authority. Creating an objective does not establish legitimate representation
of every beneficiary or affected party. Being named as a beneficiary does not
grant signing authority.

Objective v1 supports a single wallet, an organization wallet, a wallet quorum,
or designated representatives. Binding actions recover each declared signer
from an EIP-191 signature and enforce the configured threshold.

## Immutable Value Bundle

A provider proposal commits:

- the final outcome;
- any canonical monetary payment;
- exact mandatory and optional contribution needs;
- delivery and offer deadlines;
- final verification, access, and rights policies.

The provider signs the proposal. The declared objective authority signs a
separate acceptance action. Acceptance creates version 1 of the value bundle.
Amendments are intentionally unavailable in objective v1; callers receive an
explicit error rather than silently changing obligations after work begins.

The platform does not assign a universal exchange rate between money and work.
An accepted bundle means only that this provider voluntarily accepts this exact
combination of payment and verified deliverables for this objective.

## Contribution State

Each accepted contribution need defines its deliverable, purpose, recipients,
deadline, dependencies, access and rights, compensation, acceptance criteria,
evidence schema, verifier mechanism, and trust assumptions.

Each contribution offer declares its role explicitly. Records preserve that
signed role; the service does not guess a contributor's role from task text.

Work and compensation are separate state machines:

```text
work:         offered -> selected -> submitted -> verified | rejected
compensation: in_kind
              payment_pending -> paid_canonical
```

In-kind verification creates an evidence-backed, non-transferable contribution
record. It never creates a paid claim. A paid contribution reaches
`paid_canonical` only when an indexed canonical `BountySettled` event matches
the committed network, bounty contract, bounty ID, terms hash, recipient,
minimum amount, submission hash, and evidence hash.

Contribution records identify the actual role, capability, beneficiary
categories, evidence, verification mechanism and strength, compensation kind,
and completion date. They are not transferable, redeemable, purchasable proof
of work, financial promises, governance power, or a platform currency.

Final completion creates a separate non-transferable provider outcome record.
It preserves the accepted outcome commitment, beneficiaries, artifact and
submission-evidence commitments, final-verification evidence and strength, and
whether compensation was in kind or proven by canonical settlement.

## Verification Policies

Every contribution and final outcome commits its policy before work begins.
Supported mechanisms are:

- deterministic signer;
- committed verifier;
- wallet quorum;
- AI-judge quorum;
- provider acceptance;
- objective-authority approval;
- canonical autonomous bounty.

An AI-judge quorum requires at least two committed verifier wallets plus exact
provider, model, model version, prompt, rubric, benchmark, and decoding
commitments. One model response has no settlement authority.

For in-kind work, the committed mechanism signs a revision-bound verification
action. For paid work, the hosted service cannot mark the work verified or
paid: reconciliation derives both facts from the matching canonical settlement
event. A plan, signature, transaction hash, submission, verifier response, or
database row is not payment evidence.

## Graph And Readiness

Contribution dependencies form a directed acyclic graph. A need may depend on
other needs, but circular required dependencies are rejected.

`get_objective` returns the graph and an explainable readiness report. Checks
cover the accepted provider bundle, its validity, canonical funding, every
mandatory contribution, dependency completion, deadlines, final-verifier
availability, final submission, final verification, and settlement. Each failed
check includes the specific blocker and the view identifies available next
actions.

Verified supporting contributions can make an objective ready for final
execution. They cannot complete the root objective. Completion requires the
provider's final submission and its precommitted verification policy. If the
provider is paid, completion additionally requires the exact matching
`BountySettled` evidence.

## Access, Privacy, And Rights

Access, identity disclosure, evidence publication, payment evidence, and usage
rights are independent declarations. Public access is enforced by publication.
Restricted modes require a named external custodian because the hosted
coordination service and public chain do not enforce content confidentiality.

Objective v1 rejects a claim that blockchain information is private. It also
rejects restricted access metadata without external-custodian enforcement.
Never put secrets, personal data, or restricted deliverables in public terms or
evidence commitments. The platform describes declared restrictions; it does not
simulate privacy it cannot enforce.

## Agent Protocol

All mutations use plan, sign, apply:

1. Request a plan. The service validates the full content and returns a
   32-byte `commitment_hash`, required participant IDs, threshold, and current
   objective revision.
2. Required wallets sign that exact hash with EIP-191 `personal_sign`.
3. Submit the unchanged plan and signatures.
4. Read the objective again before planning the next action.

Changed content, wrong wallets, duplicate signers, invalid thresholds, stale
revisions, and replayed actions fail closed. Postgres updates use
compare-and-swap on the objective revision, so concurrent writers cannot
silently overwrite one another.

Core MCP tools:

```text
plan_objective_creation
create_objective
list_objectives
get_objective
plan_objective_action
apply_objective_action
reconcile_objective
```

REST equivalents:

```text
POST /v1/objectives/creation-plans
POST /v1/objectives
GET  /v1/objectives
GET  /v1/objectives/{objective_id}
POST /v1/objectives/{objective_id}/action-plans
POST /v1/objectives/{objective_id}/actions
POST /v1/objectives/{objective_id}/reconcile
```

Action objects are tagged by `kind`: `add_provider_proposal`,
`accept_provider_proposal`, `offer_contribution`,
`select_contribution_offer`, `submit_contribution`, `verify_contribution`,
`submit_final_outcome`, or `verify_final_outcome`. Use the hosted OpenAPI schema
for the exact nested request types.

Reconciliation is permissionless and accepts no caller-supplied payment claim.
The service rebuilds evidence from its confirmed canonical event index. Anyone
may trigger reconciliation; nobody can use it to choose a verdict or recipient.

## V1 Boundaries

- Coordination state is durable Postgres data, not an objective smart contract.
- Monetary custody and settlement stay in isolated canonical bounty contracts.
- Wallet-backed participants are required for binding objective actions.
- Accepted-bundle amendments are disabled until affected-party consent and
  version migration semantics are designed and tested.
- Restricted content requires an external custodian.
- There is no platform token or universal conversion between labor and money.
- Funding, work, organization, verification, and authority remain distinct
  signals; none automatically purchases the others.

These limits preserve the existing payment evidence rules while objective
coordination evolves independently.
