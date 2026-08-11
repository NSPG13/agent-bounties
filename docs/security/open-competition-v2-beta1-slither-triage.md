# Open Competition V2 Beta1 Slither Triage

Tool: `slither-analyzer 0.11.5`, 101 detectors, V2 factory dependency graph.
Generated JSON stays in `target/tmp/open-competition-v2-slither.json`.

This is maintainer static-analysis evidence, not the three independent R4
reviews required for graduation.

## Actioned

| Detector | Resolution |
| --- | --- |
| `return-bomb` on ERC-1271 | Fixed. `_isValidSignatureNow` now requests and copies at most one 32-byte word, checks `returndatasize`, and never materializes unbounded return bytes. `testErc1271ReturnDataIsCappedAndCannotBombRelay` covers a 65,536-byte malicious response. |

## Reviewed Findings

| Detector | Triage |
| --- | --- |
| `arbitrary-send-erc20` in `fundFromFactory` | False positive. Only the immutable factory may call it; direct creation fixes `contributor = msg.sender`, while authorized creation uses USDC EIP-3009 bound to exact sender, recipient, amount, nonce, and validity. The factory never becomes token spender or custodian. |
| `reentrancy-no-eth` / `reentrancy-benign` in funding | Covered by the contract-wide `_reentrancy` state guard. Every external funding entry acquires it before USDC or EIP-3009 calls. Base deployments pin native USDC and exact post-transfer balance deltas. Malicious callback and accounting tests remain required. |
| `reentrancy-benign` in factory initialization | The external target is a just-created deterministic clone of the release implementation. Both public creation methods hold the factory guard; initialization is one-shot, and canonical registration occurs in the same transaction. |
| `events-maths` in clone initialization | Configuration is emitted by the factory as canonical economics, verification, and policy events immediately after initialization and before initial funding. The clone cannot be initialized outside the factory. |
| `missing-zero-check` for gateways | The adapter constructor requires gateway bytecode, expected verifier bytecode, and the exact unfrozen route. Base and Base Sepolia additionally require exact canonical addresses. |
| `timestamp` | Intentional protocol deadlines. Boundary semantics are explicit: proof submission is inclusive at the deadline; finalization and expiry require a later timestamp. Dedicated exact-boundary tests cover each transition. |
| `assembly` | Limited to deterministic minimal-proxy deployment, bounded ERC-1271 output, and strict ECDSA parsing. Each block has focused tests and no arbitrary storage access. |
| `low-level-calls` | Intentional fail-closed boundaries for optional-return ERC-20 behavior, SP1 proof rejection mapping, gateway liveness, and bounded ERC-1271 validation. Return lengths and status are checked. |

## Re-run

```powershell
$env:Path = "$PWD\.tools\foundry;$env:APPDATA\Python\Python312\Scripts;$env:Path"
slither contracts/base-escrow/src/OpenCompetitionBountyFactoryV2Beta1.sol `
  --compile-force-framework foundry `
  --foundry-out-directory out `
  --exclude-dependencies `
  --json target/tmp/open-competition-v2-slither.json
```

Any new high or medium finding blocks the release gate until it is fixed or
documented with a test-backed rationale.
