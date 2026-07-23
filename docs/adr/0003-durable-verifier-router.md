# ADR 0003: Durable policy-bound verifier routing

## Status

Proposed for the bounded-agent wallet migration tracked in issues #527 and #571.

## Decision

The existing Base bounded-agent wallet remains the custody account. Its owner makes one final policy update that preserves the live action mask, verifier-set commitments, and spending caps while:

1. rotating the delegate from the externally signed relay wallet to the protected production keeper that already holds the deployment signer;
2. replacing the single version-specific deterministic verifier with one stable `PolicyBoundVerifierRouter` address; and
3. extending the policy validity horizon so routine operation does not expire back into an owner-signature requirement.

The delegate rotation is necessary because the current delegate is operated through externally supplied signed relay envelopes. Keeping it would preserve the recurring signature dependency this migration is intended to remove. The keeper cannot exceed the unchanged on-chain per-action, period, lifetime, bounty-target, action, or verification-mode limits.

The router is not upgradeable and cannot move funds. It routes `IAgentBountyVerifier.verify` by the bounty's immutable `policyHash` to one append-only implementation record. Each active record pins the implementation address and runtime code hash forever. Existing active records cannot be replaced, cancelled, or redirected.

The protected keeper is also the router registrar and may propose a new policy without the wallet owner. New policies activate only after a seven-day public delay. The wallet owner is the guardian and may veto a pending proposal during that delay, but routine activation requires no owner signature. The deployment has one bounded bootstrap slot so the initial routed profitable V3 policy can be activated atomically with deployment.

Routed implementations receive the canonical parent bounty address explicitly from the router and must commit their router and policy hash as immutable metadata. The router checks that metadata and the runtime code hash when registering and on every verification call.

## Consequences

- Routine bounty creation, funding, claiming, and submission continue without owner signatures inside the unchanged bounded-wallet caps.
- Future verifier versions do not require another wallet policy signature.
- A new policy cannot alter the payout rules of an already funded bounty because its `policyHash -> implementation` record is immutable.
- A compromised keeper cannot activate a new verification policy immediately; the seven-day delay provides a public veto window.
- A compromised keeper also remains financially bounded by the existing wallet action and spend caps and cannot call arbitrary contracts or withdraw custody funds.
- The guardian is not a routine approval authority. Its signature is needed only for veto, custody recovery, or an intentional future change to the financial/action caps.
- V4 and later deterministic policies should implement the routed verifier interface instead of requesting another bounded-wallet verifier change.

## Evidence boundary

Source, CI, a predicted address, a policy proposal, or an owner signature is not deployment or funding evidence. Runtime bytecode, immutable router state, the confirmed `PolicyConfigured` event, canonical bounty creation/funding events, and ultimately `BountySettled` remain authoritative.
