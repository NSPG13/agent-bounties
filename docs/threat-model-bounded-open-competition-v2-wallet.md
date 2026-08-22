# Threat model: bounded Open Competition V2 reserve wallet

## Scope and assets

The reserve wallet holds operator USDC used only for reviewed Open Competition
V2 Beta3 creations. The assets are the uncommitted USDC balance, creator refund
rights in canonical competitions, policy integrity, candidate commitments,
spending counters, owner recovery authority, and evidence that a competition is
canonically active.

The initial owner is
`0x884834E884d6e93462655A2820140aD03E6747bC`. The initial authorization is
77.668098 USDC, with exact 3.04-USDC creations and a 30.40-USDC daily cap. The
delegate holds no reserve USDC and cannot withdraw, transfer, settle, verify, or
change policy.

## Trust boundaries

- The owner approves deployment, initial funding, policy changes, revocation,
  refund recovery, and two-step ownership transfer.
- The delegate selects only from owner-approved content commitments and pays
  gas to submit creation calls.
- The bounded wallet trusts the immutable canonical V2 Beta3 factory and its
  immutable settlement token.
- Each created competition holds committed funds under its own immutable
  protocol state machine. Active escrow is outside the reserve's immediate
  recovery balance.
- Private ranking and indexer reconciliation remain operational inputs. They do
  not expand the delegate's on-chain authority.

## Abuse and failure analysis

| Threat | Control | Residual consequence |
|---|---|---|
| Delegate key compromise | Exact preapproved creation commitments, policy validity, daily/lifetime caps, one-use commitments, immutable factory, and no arbitrary-call or transfer method | Attacker may activate still-approved candidates until a cap is reached or the owner revokes |
| Delegate refuses recovery | Recovery is owner-only and does not call the delegate or hosted service | None for uncommitted funds; Base must still be usable |
| Ordinary-wallet custody loss | Reserve USDC stays in the bounded contract; delegate receives gas only | Owner-key loss remains an owner custody risk |
| Unauthorized withdrawal | No delegate withdrawal selector; recovery requires owner plus prior revocation | Compromised owner can recover uncommitted funds |
| Policy reconfiguration resets caps | Lifetime spend persists across versions; period duration cannot change; same-period spend is synchronized before update | Owner can deliberately increase future caps in a separately signed transaction |
| Duplicate or replayed creation | Commitment binds chain, immutable factory, full parameters, and nonce; global used flag survives policy versions | None unless commitment collision breaks keccak256 |
| Allowance theft | Wallet approves only the predicted competition for exactly 3.04 USDC and resets allowance to zero after canonical activation | A defective approved factory release remains a release-review risk |
| Fake or partially funded activation | Post-call checks require predicted address, canonical-factory registration, owner identity, token, target, funded amount, active status, and zero allowance | Canonical factory or token defects require migration |
| Front-run deterministic wallet deployment | Unfunded deployment requires `msg.sender == owner`; funded EIP-3009 execution sends funds only to the owner/policy-bound predicted wallet | A copied valid authorization may be relayed first, but cannot redirect funds |
| Duplicate initial funding | Funded factory paths reject an already deployed wallet and token balance deltas must match exactly | Owner can still intentionally transfer tokens directly; UI must warn against bypassing the reviewed flow |
| Malicious or fee-on-transfer token | Factory and recovery balance deltas must match; production factory pins canonical Base USDC | Noncanonical test tokens are out of production scope |
| Reentrancy | Wallet and factory state-changing external paths use independent guards; spend and commitment are recorded before factory interaction | Canonical USDC/factory assumptions still require bytecode pinning |
| Verifier disappears | After revocation, owner invokes the canonical unavailable-verifier cancellation and pulls the creator refund | Keeper reward follows the competition contract and is returned when the reserve is the caller |
| Competition expires without a winner | After revocation, owner may expire it, pull the solver refund, then recover the combined refund and keeper reward | If a third party performs expiry first, that party earns the keeper reward; a BestScore competition with a valid leader must finalize and its payout is not recoverable |
| Owner mistypes successor | Two-step transfer requires acceptance by the pending owner | A malicious accepted successor controls uncommitted recovery and policy |
| Owner attempts early clawback | No path cancels a healthy active competition before its proof deadline | Capital remains committed until settlement, canonical cancellation, or expiry by design |

## Recovery invariant

After `revokePolicy()` succeeds:

1. no delegate creation can succeed;
2. `recoverUncommitted()` transfers the reserve's full USDC balance only to the
   current owner;
3. only canonical competitions created and fully activated by the reserve can
   enter its refund helpers;
4. refunds are first paid by the competition to the reserve, then recovered by
   the owner; and
5. settled solver or keeper payouts are never described or treated as
   recoverable reserve funds.

## Launch gates

This is an R4 value-bearing change. Base mainnet deployment and funding require
independent review, focused and full contract tests, exact compiler and bytecode
pinning, Base Sepolia rehearsal of creation/revocation/recovery/concurrency,
explicit owner review of the predicted address and policy hash, and a one-item
mainnet canary. A source commit or passing local test is not deployment evidence.
