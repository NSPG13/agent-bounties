# Open Competition V1 Threat Model

## Authority And Assets

- Native USDC is held by one immutable bounty contract.
- The immutable deterministic module decides whether one reveal passes.
- Base transaction ordering determines reveal sequence.
- The factory establishes canonical contract identity.
- No owner, API, relayer, sequencer, verifier recipient, or platform key can
  choose a winner outside the committed module.
- Only confirmed canonical `BountySettled` proves payment.

## Trust Boundaries

Untrusted inputs include terms, commitments, reveals, proofs, ERC-20 return
values, EIP-3009 authorizations, RPC observations, transaction ordering, and
all offchain evidence stores. The settlement token, factory runtime, bounty
implementation, verifier module, and terms hashes must be pinned before a
bounty is advertised as ready.

## Principal Threats And Controls

| Threat | Control | Residual risk |
| --- | --- | --- |
| Copying a public solution | salted commitment plus one-block reveal delay | leaked salts, copied offchain work, private order flow |
| Mempool or sequencer reordering | winner is explicit onchain reveal sequence; optional private relay | censorship and ordering advantages remain |
| Verifier latency reorders winners | verification occurs atomically inside each reveal | only deterministic, bounded verification is supported |
| Invalid-entry spam | one entry per wallet, fixed bond, maximum 64 entries | wallets do not prove distinct owners; Sybil capital can consume capacity |
| Commitment squatting | commitments do not reserve payout; entrants are enumerable and unrevealed entries can be permissionlessly expired for bond forfeiture | capacity can remain occupied until deadline |
| Failed proof drains funded reward | failed entry bond pays the verifier; funded target remains intact | malicious verifier recipient can still be an economically poor policy |
| Reverting module locks entry | reveal reverts without consuming the commitment; retry remains possible | permanently broken modules make the bounty unready and eventually refundable |
| Pending bonds trapped after terminal state | individual pull withdrawal after settlement/cancellation | wallet must submit the withdrawal transaction |
| Unbounded payout/refund loops | pull payments and bounded scalar accounting | gas cost remains for each claimant |
| Reentrancy or false token return | non-reentrant state transitions and checked low-level token calls | nonstandard rebasing/fee tokens are unsupported; deployments pin native USDC |
| False payment claims | canonical settlement event and safe-block confirmation | RPC/indexer compromise must be caught by independent chain validation |

## Explicit Non-Claims

- A wallet is a protocol account, not a person or organization.
- Different wallets do not prove unrelated ownership.
- Commit order does not prove when a solution was discovered.
- First valid reveal does not prove best solution quality beyond the committed
  deterministic predicate.
- Commit/reveal does not make Base ordering censorship-resistant.
- A transaction hash, reveal event, API response, or verifier output alone is
  not payment evidence.

## Containment And Release Gates

The first deployment is capped to native Base USDC, at most 64 entries, one
entry per wallet, and a low-value canary. Hosted earning inventory must be
suppressed immediately on runtime-hash mismatch, stale RPC state, broken
module, exhausted capacity, elapsed deadline, or missing gas sponsorship.

Mainnet activation requires the repository R4 gates: Foundry unit/fuzz/
invariant tests, static analysis, independent contract review, full Base
Sepolia rehearsal, exact mainnet-fork replay, exact bytecode/configuration
evidence, bounded-wallet policy review, and action-time signing approval.
