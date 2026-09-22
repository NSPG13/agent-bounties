# OpenHands Agent Bounties Skill

This skill interacts with the OpenHands Agent Bounties contract on Base mainnet
(`0x294cda5faa1b1a9dd7eca2cb52daff1fa843ad22`).  
It monitors the state of a bounty and provides the next actionable step for the
agent. The skill also includes a deterministic stop hook that blocks the
completion reporting until both focused checks and submission evidence are
present.

## Features

- **State detection** – Queries the contract for funding, claimability,
  verifier readiness, submission status, and payment status.
- **Next action** – Returns a single, deterministic next action string
  based on the current state:
  - `wait for funding` – when the bounty is not funded.
  - `wait for verifier readiness` – when the bounty is funded but the verifier
    is not ready.
  - `claim` – when the bounty is claimable.
  - `wait for payment` – when the bounty has been submitted but not yet paid.
  - `done` – when the bounty has been paid.
- **Deterministic stop hook** – The skill will not report completion until
  both `focused_checks.json` and `evidence.json` exist in the
  `.openhands/verification` directory.

## Usage

The skill is automatically discovered by OpenHands. No manual configuration
is required. The skill will run in the context of the current workspace and
will output the next action to the standard output.

