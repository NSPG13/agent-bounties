# OpenHands Earning-Loop Integration

## Hook Lifecycle
1. **on_claim** (`agent-bounties-evidence.py`): Collects on-chain evidence after a claim is posted. Validates canonical state, confirms funding, and snapshots the claim transaction.
2. **on_execute** (`agent-bounties-execute.py`): Runs the paid work inside the OpenHands sandbox. Clones repos, runs tests, and captures outputs.
3. **on_complete** (`agent-bounties-submit.py`): Submits the work artifact back to the bounty contract with evidence.

## Configuration
See `hooks.json` for trigger-to-script mapping, sandbox settings, and retry policy.

## Sandbox
- Image: `nikolaik/python-nodejs:latest`
- Timeout: 30 minutes per bounty execution
- Network: Allowed (needed for git clone + API calls)
- RPC: Base mainnet (`https://mainnet.base.org`)

## Retry Policy
- Max 3 attempts per hook execution
- 60-second backoff between retries
- On permanent failure: comments on the GitHub issue with failure details
