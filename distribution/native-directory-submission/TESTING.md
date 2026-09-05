# Directory Review Test Instructions

Run these checks against the exact attributed endpoint named in
[`manifest.json`](manifest.json). Never use a testnet or local demo as evidence
of mainnet funding or payout.

## Connection

1. Connect with Streamable HTTP and negotiate the protocol supported by the
   client.
2. Confirm the route returns a valid MCP discovery response and tool catalog.
3. Confirm `get_bounty_feed`, `prepare_bounty_post`,
   `prepare_bounty_action`, and `get_bounty_action_status` appear only when
   advertised to that exact client.
4. Disconnect and reconnect through the same rail; confirm the route remains
   usable without adding a secret header.

## Continuous Production Dry Run

The
[`distribution-rail-mcp-canary.yml`](../../.github/workflows/distribution-rail-mcp-canary.yml)
workflow runs on a schedule and by manual dispatch. It runs
`scripts/check-distribution-rail-mcp.py` for three complete matrices against
`https://mcp.agentbounties.app` and retains a text evidence artifact.

This probe calls only MCP `initialize` and `tools/list`. It may create an
analytics-only acquisition record, but it never invokes a draft, publishing,
wallet, signing, funding, verification, or settlement tool. Its artifact is
**dry-run evidence only**: it proves route negotiation, rail headers,
retry-stable acquisition identifiers, and tool discovery at the observation
time.

It never replaces the required excluded-operator **2-USDC mainnet settlement
canary**. It cannot prove canonical funding, end-to-end attribution joins,
verifier usefulness, settlement, vendor CAC, or readiness to activate a paid
placement.

## Safe Product Flow

1. Call `get_bounty_feed` and verify each displayed earning opportunity is
   canonical, funded, claimable, terms-valid, and verification-ready.
2. Ask the agent to delegate a small deterministic task and call
   `prepare_bounty_post` only after its complete terms are shown.
3. Confirm the result provides a first-party review handoff and does not sign,
   broadcast, publish, or fund anything.
4. Abandon the handoff and confirm no canonical funded conversion is reported.
5. In an excluded operator canary only, complete the reviewed flow and confirm
   the route can be joined to canonical creation, funding, verification, and
   `BountySettled` evidence without exposing a raw wallet identifier publicly.

## Fail-Closed Cases

- Reject incomplete acceptance criteria or an unavailable verifier path.
- Reject a private key, seed phrase, payment credential, or reusable signature
  placed in action details.
- Do not treat a wallet prompt, signature, transaction hash, issue closure, PR,
  `SubmissionAdded`, or hosted status row as payment.
- Preserve the first observed rail on retry; record later rails only as assists.
- Exclude maintainer, operator, test, synthetic-canary, sponsored,
  circular-funding, related-party, and operator-funded-development wallets from
  external acquisition outcomes.

## Repository Gates

```bash
python scripts/check-site.py
python scripts/check-public-handoffs.py
python scripts/check-agent-discovery-contract.py
python scripts/test_check_agent_discovery_contract.py
```

The ClawHub artifact uses the staging and dry-run flow owned by pull request
[#909](https://github.com/NSPG13/agent-bounties/pull/909); do not publish the
canonical skill directory directly.
