# Local Delegate Wallet

The local delegate is the immediate provider-neutral signer for a bounded Agent
Bounties wallet. It replaces the unavailable MetaMask Agent Wallet early-access
dependency without importing a MetaMask owner key or changing the on-chain
policy.

The delegate key is generated locally, encrypted as a Web3 Secret Storage v3
keystore, and stored outside the repository. Its random keystore password is
protected by Windows DPAPI for the current Windows user. The directory ACL is
replaced so only that user can read it. The bounded contract wallet, not the
local process, enforces the network, token, factory, verifier, action, bounty,
rate, lifetime, and expiry limits.

## Install

```powershell
python -m pip install -r scripts/requirements-wallet.txt
python scripts/local_delegate_wallet.py init
python scripts/local_delegate_wallet.py status
```

`init` prints only the public delegate address. It never prints the private key
or keystore password. By default, private state lives at:

```text
%LOCALAPPDATA%\AgentBounties\delegate
```

Use the printed address as `--delegate` when generating the one-time owner plan:

```powershell
python scripts/plan_bounded_agent_budget.py `
  --owner 0xOWNER `
  --delegate 0xDELEGATE
```

The owner reviews and signs that exact policy once. After the factory and wallet
are deployed and the inspection succeeds, bind this signer to the observed
wallet. Copy the owner and policy hash from the reviewed activation plan:

```powershell
python scripts/local_delegate_wallet.py bind `
  --wallet 0xBOUNDED_WALLET `
  --expect-owner 0xOWNER `
  --expect-policy-hash 0xPOLICY_HASH
```

Binding is intentionally immutable. Rotate by creating a new delegate directory
and installing a new owner-approved policy.

## Execute One Action

Generate a same-state action plan with the existing planner:

```powershell
python scripts/plan_bounded_agent_action.py fund `
  --wallet 0xBOUNDED_WALLET `
  --bounty 0xCANONICAL_BOUNTY `
  --amount-usdc 2.01 `
  --expect-owner 0xOWNER `
  --expect-delegate 0xDELEGATE `
  --expect-policy-hash 0xPOLICY_HASH
```

Simulate and inspect gas without decrypting or signing:

```powershell
python scripts/local_delegate_wallet.py execute-plan `
  --plan target/bounded-agent-action-plan.json
```

Broadcast autonomously after the agent's own task-selection policy approves the
work:

```powershell
python scripts/local_delegate_wallet.py execute-plan `
  --plan target/bounded-agent-action-plan.json `
  --broadcast
```

The local signer re-inspects the wallet and bounty, verifies the original safe
block is canonical and at most five minutes old, re-derives the exact action
calldata, simulates it, and enforces local gas caps. It rejects arbitrary
targets, arbitrary calldata, ETH value, a changed policy, stale state, and
non-canonical bounties. The delegate address needs a small Base ETH reserve for
gas; it does not need USDC because USDC remains in the bounded wallet.

## Operational Boundaries

- Back up the encrypted keystore, DPAPI blob, and Windows account recovery
  material together. DPAPI-protected data is intentionally not portable by
  itself.
- Never commit the private state directory or send its files to an API, MCP
  server, issue, PR, or bounty.
- Owner revocation remains the emergency stop. Revoke on-chain before retiring
  or rotating a delegate.
- A successful transaction applies one bounded action. Only reconciled
  canonical events prove funding, claim, submission, or payout.
- CDP, Circle, Turnkey, an HSM, or MetaMask Agent Wallet can later replace this
  adapter by using the same public delegate and action-plan boundaries.
