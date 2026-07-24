# Standing Meta V4 threat model

Status: design, local-test, and Base-mainnet fork evidence only. V4 is not deployed and is not ready to earn. The current fork test deploys the exact staged graph against live Base USDC and the official VRF coordinator, creates and funds a fork-only native subscription, and authorizes both sortition coordinators. Independent review, the real Base Sepolia rehearsal, deployed bytecode/configuration evidence, protected repository environments, a funded real VRF subscription, authorized real consumers, and gas sponsorship remain release gates.

## Security claims

V4 provides anonymous economic separation. It does not provide identity, proof-of-personhood, KYC, organizational attestation, or proof that two wallets have unrelated owners. Chainlink VRF chooses among a frozen set of eligible wallets; it does not inspect work, decide a verdict, or authorize payment.

The only payment proof is a confirmed canonical `BountySettled` event emitted by the exact bounty contract. An API response, signature, VRF result, verdict, transaction hash, or hosted database row is not payment proof.

## Assets and trust boundaries

- Parent escrow: exactly 2.01 USDC, paying 2.00 USDC to the parent solver and 0.01 USDC to the finalizer.
- Child escrow: exactly 1.00 USDC, paying 0.99 USDC to the child solver and 0.01 USDC to the verifier module.
- Role stake: exactly 5 USDC per wallet and role, one ticket per wallet and no greater odds for extra stake.
- Appeal bond: 0.10 USDC.
- The platform funds required gas and the Chainlink VRF 2.5 native-token subscription. These are operational dependencies, not judgment authority.
- The immutable controller is configured once. A wrong initial configuration is permanent, so deployment evidence and independent review are mandatory.
- Components are deployed in a reviewed staged sequence, including a dedicated claim-restricted V4 child factory, then an immutable `StandingMetaV4Bundle` validates and records their exact wiring. This stays below EIP-170 and EIP-3860 limits; the bundle is not evidence that VRF funding or consumer authorization succeeded.

## Threats and controls

| Threat | Control | Residual risk |
|---|---|---|
| One owner operates many anonymous wallets | Fixed stake, one ticket per wallet/role, seven-day activation, frozen candidate sets, random assignment, slashing | Wealthy or coordinated actors can still control multiple wallets; unrelated ownership is not proven |
| Project chooses favorable verifiers | Candidate set is frozen before one Chainlink VRF request; request ID and commitment are bound; no reroll or fallback randomness | VRF availability and subscription funding are dependencies |
| Candidate joins after seeing a target | Child solver candidates are the already-active, available pool snapshotted inside `claimAndCreateChild` | Availability may change after the snapshot; ranking activation and claims still fail closed |
| Enrollment delay blocks fast work | There is no per-bounty enrollment window; the VRF request is made atomically with the parent claim | New wallets still wait seven days before becoming active, which is a Sybil-cost control |
| Selected solver does not respond | One ranking is reused and a permissionless promotion becomes available after two minutes | A two-minute liveness delay remains for each nonresponsive rank; an intermittently connected selected wallet can miss the assignment |
| Unselected wallet directly reserves the child | The specialized V4 child has no generic claim path; only the immutable child factory can activate the currently ranked wallet | A selected wallet can still fail to respond and trigger the bounded promotion delay |
| Primary verifier does not respond | Primary plus three ranked backups; unavailable primaries lose 0.01 USDC | Exhausting all backups times out rather than accepting a platform verdict |
| Primary judgment is disputed | Solver may appeal rejection and creator may appeal acceptance; five-wallet jury, three-vote threshold | Subjective judgment can still be wrong or coordinated |
| Uncontested verdict waits unnecessarily | The only eligible appellant may waive the remaining appeal window, finalizing immediately | Without a waiver, the full appeal window remains available |
| Jury result is already decisive | Three matching votes can be finalized immediately | A split or missing quorum waits until timeout and then fails closed |
| Callback griefing or out-of-order fulfillment | Callback only stores request-bound randomness and never performs downstream settlement; ranking is derived separately | A late callback is unusable and requires recovery, never platform randomness |
| Verification starts too late to finish an appeal | Case opening requires enough time for both VRF windows, all four primary response windows, appeal and voting windows, plus a transaction buffer; primary randomness has an explicit fail-closed timeout | A case not opened promptly can time out the child and require a new round |
| Reroll or replay | One request per commitment, no cancellation, no replacement request, request-ID binding | A permanently failed request cannot be rescued inside that round |
| Atomic preparation race | Terms publication, child creation/funding, active-pool snapshot, VRF request, round binding, and parent claim occur in one transaction | The transaction can revert for gas, authorization, pool-size, or subscription failures |
| Fake profitable economics | Exact integer micro-USDC checks and deterministic parent predicate require 2.00 minus 1.00 equals 1.00 USDC | This is successful-settlement onchain margin, not net profit; labor, compute, tax, failure, and opportunity cost remain |
| Open parent race externalizes child cost to losers | V4 remains `vrf_assigned_child`; it is not routed through first-valid open competition | A future meta competition needs capped reimbursement or platform-funded children and a separate R4 review |
| Gas or VRF reserve is depleted | Ready-to-earn is fail-closed unless sponsorship and VRF reserve are observed and consumers authorized | Observation can become stale; monitoring must suppress new earning immediately |
| Verifier signs contradictory verdicts | Valid contradictory EIP-712 signatures are slashable by 0.10 USDC | A subjective overturned judgment alone is not called fraud |
| Appeal griefing | Fixed bond, one appellate round, bounded jury and deadlines | An appellant can still impose delay and transaction costs |
| Token or canonical-contract mismatch | Parent verifier checks exact token, factories, registries, modules, terms, round, timestamps, hashes, deadlines, and settled child state | Bugs in immutable validation require cancellation/migration rather than upgrade |

## Release gates

V4 must remain absent from ready-to-earn and verification-job views until every item below has current evidence:

1. Exact source revision, compiler settings, bytecode hashes, constructor arguments, and immutable getters.
2. Independent contract review with findings resolved or explicitly accepted.
3. Base Sepolia rehearsals for unappealed/waived, upheld, overturned, timeout, cancellation, refund, child settlement, parent settlement, and canonical `BountySettled` evidence.
4. Base-mainnet fork test using current USDC and official Chainlink coordinator configuration.
5. Authorized VRF consumers, funded native-token subscription reserve, and measured callback latency.
6. Gas sponsorship reserve and a successful sponsored action rehearsal.
7. At least eight eligible verifier wallets and at least three eligible child-solver wallets after exclusions.
8. A bounded-wallet policy review covering stake, child funding, claim bonds, appeal bonds, and transaction targets.

## Latency policy before immutable deployment

The reviewed constants intentionally distinguish successful-path latency from
failure and fairness deadlines:

- per-bounty solver enrollment: zero;
- VRF request confirmations: three; successful fulfillment proceeds
  immediately, while two hours is only the fail-closed outage deadline;
- selected child-solver response: two minutes per ranked wallet;
- primary and each of three backups: 30 minutes;
- appeal filing: four hours, with immediate waiver by the only eligible
  appellant;
- appellate voting: two hours, with immediate finalization after three matching
  votes;
- child and parent verification envelope: 24 hours;
- global stake activation and unbonding: seven days, paid once per role ticket
  rather than once per bounty.

Shortening the one-block Open Competition V1 reveal separation or the
three-confirmation VRF floor would weaken copy-resistance or randomness
security rather than remove idle application latency. Those values therefore
remain unchanged. Independent review must evaluate the two-minute assignment
window for missed-wakeup risk before deployment.
