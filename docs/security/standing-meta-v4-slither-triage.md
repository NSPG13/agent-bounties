# Standing Meta V4 Slither triage

Date: 2026-07-22

Tool: Slither 0.11.5

Compiler: solc 0.8.26, optimizer enabled with 200 runs, Cancun EVM

Scope: V4 contracts and their anonymous staking, sortition, and appeal dependencies

This is maintainer static-analysis evidence, not the independent contract review required by R4.

## Result

- High: 0
- Medium: 29 detector results
- Low: 66 detector results
- Informational: 4 detector results

The medium results were reviewed as follows:

| Detector | Count | Triage |
|---|---:|---|
| `reentrancy-no-eth` | 12 | V4 state-changing entry points that cross token/controller/module boundaries use contract-wide `nonReentrant` locks. The settlement token and controller are also pinned by immutable wiring. Slither does not recognize the local guard implementation and reports guarded paths, including `StandingMetaChildV4.claimAuthorized`. |
| `incorrect-equality` | 6 | Strict equality is intentional for exact USDC funding, exact commitment binding, unique request initialization, and fixed V4 economics. Accepting surplus funding would break conservation/accounting assumptions. |
| `unused-return` | 6 | Stake-release amounts and unused tuple fields are deliberately not decision inputs. The state transition is checked through the controller/pool and later conservation assertions. |
| `uninitialized-local` | 4 | Solidity zero initialization is intentional for counters and the one-time remainder recipient flag. |
| `divide-before-multiply` | 1 | `share = amount / voteCount` followed by `remainder = amount - share * voteCount` intentionally computes equal integer shares plus the exact remainder without losing USDC base units. |

Low timestamp findings are expected in a deadline-driven protocol and use multi-minute/hour/day windows, not timestamp randomness. Loop findings are bounded by the 64-ticket pool and five-member jury. Assembly findings are limited to established signature recovery and deterministic deployment patterns.

## Changes made during triage

- Added contract-wide reentrancy locks to all mutable appeal-verifier and parent-factory entry points.
- Added primary-VRF and appeal-VRF fail-closed timeout handling.
- Required enough remaining bounty time to complete both VRF windows, every primary backup window, one appeal, voting, and a settlement buffer.
- Added a dedicated claim-restricted V4 child and child factory so an unselected wallet cannot reserve the child through the generic bounty claim path.
- Tightened the one-time child-factory configuration to validate the parent factory's base factory, child factory, and appeal verifier before it can become immutable.
- Required a settled child before the parent can begin its own verification window.

These changes and this triage must be reviewed independently before any mainnet deployment flag is changed.
