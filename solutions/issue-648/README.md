# Evidence Shape & Policy Validator Helper: Issue #648 (MCP Interoperability Task)

**Target Issue:** `NSPG13/agent-bounties#648`  
**Module:** `solutions/issue-648`  
**Role:** Non-Authoritative Client-Side Policy & Shape Validator Helper  

---

> [!NOTE]
> **Boundary of Authority:** This module is an offline client-side validator helper for syntactic payload-shape, exact decimal/micro-USDC math, and local policy sanity checks. It does **not** constitute canonical on-chain verification, substitute for parent/child coordination events, or authorize registration, bonds, child funding, or bounty settlement.

---

## 📋 Evaluated Policy Controls

| Control ID | Description | Rule Conformance & Sanitization |
| :--- | :--- | :--- |
| `CONTROL_01_PARTICIPANT_INDEPENDENCE` | Verifies parent solver and child solver are independent participants. | Distinct valid Base EVM hex addresses (`0x...`). Rejects identical participants and all-zero (`0x000...`) addresses. Confirms distinct supplied addresses. |
| `CONTROL_02_MARGIN_AND_FUNDING` | Validates exact micro-USDC funding target and retained gross margin. | Normalizes validated amounts to exact integer micro-USDC (`1 USDC = 1,000,000 micro-USDC`) via exact `Decimal` representation. Rejects fractional micro-USDC (sub-cent precision beyond 6 decimals), negative amounts (`-0.0000001`), amounts exceeding bounds (`1000000.0000001`), and `Decimal('1e1000000')` exponent overflow. Enforces `child_target >= 1.000000 USDC` and `(parent_reward - child_target) >= 1.000000 USDC`. |
| `CONTROL_03_VERIFIER_QUORUM` | Validates strict integer quorum threshold, framework, and immutable policy hash. | Framework must equal `sandboxed_regression_v1` with `quorum >= 2` (strict integer, rejecting floats like `2.5`, strings, and null). Router and immutable policy hash must match canonical constants. |
| `CONTROL_04_LANE_READINESS_AND_SETTLEMENT` | Validates profile approval, non-zero tx hash, and receipt structures. | Requires strict boolean `child_profile_approved == True` (rejecting string `"false"`). Requires canonical `BountySettled` event, strict boolean `settled == True`, non-zero `tx_hash`, valid recipient, and child contract. Settlement receipt shape accepted. |

---

## 🧪 Unit Test Suite Execution

The accompanying unit test suite (`test_solution.py`) contains 23 deterministic tests covering valid conformance and negative control failures:

```bash
python3 -m unittest test_solution.py -v
# Ran 23 tests in 0.002s -> OK
```
