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
| `CONTROL_02_MARGIN_AND_FUNDING` | Validates exact micro-USDC funding target and retained gross margin. | Normalizes validated amounts to exact integer micro-USDC to prevent binary floating-point subtraction precision artifacts (`2.01 - 1.01 = 1.000000`). Strictly rejects booleans, strings, `NaN`, infinities, and large integers (`10**400` overflow guard). Enforces `child_target >= 1.000000 USDC` and `(parent_reward - child_target) >= 1.000000 USDC` within bounded limits. |
| `CONTROL_03_VERIFIER_QUORUM` | Validates strict integer quorum threshold, framework, and immutable policy hash. | Framework must equal `sandboxed_regression_v1` with `quorum >= 2` (strict integer, rejecting floats like `2.5`, strings, and null). Router and immutable policy hash must match canonical constants. |
| `CONTROL_04_LANE_READINESS_AND_SETTLEMENT` | Validates profile approval, non-zero tx hash, and receipt structures. | Requires strict boolean `child_profile_approved == True` (rejecting string `"false"`). Requires canonical `BountySettled` event, strict boolean `settled == True`, non-zero `tx_hash`, valid recipient, and child contract. Receipt shape accepted. |

---

## 🧪 Unit Test Suite Execution

The accompanying unit test suite (`test_solution.py`) contains 19 deterministic tests covering valid conformance and negative control failures:

```bash
python3 -m unittest test_solution.py -v
```

### Assertions Covered (19 Tests):
1. **Full Conformance:** Deterministic passage of all 4 controls on valid evidence.
2. **Exact Float Subtraction Normalization:** `parent=2.01` and `child=1.01` correctly passes with exact 1 USDC margin.
3. **Exact Margin Boundary:** Validates boundary (1.000000 USDC passes, 0.999999 USDC fails).
4. **Large Integer Overflow Guard (Parent):** `10**400` fails cleanly without `OverflowError`.
5. **Large Integer Overflow Guard (Child):** `10**400` fails cleanly without `OverflowError`.
6. **Large Integer Overflow Guard (Receipt):** `10**400` fails cleanly without `OverflowError`.
7. **Identical Participants Rejection:** Rejects parent == child.
8. **All-Zero Address Rejection:** Rejects `0x0000000000000000000000000000000000000000`.
9. **String & Boolean Amounts Rejection:** Rejects `True`/`False` or strings as monetary values.
10. **NaN & Infinity Rejection:** Rejects non-finite floating point amounts.
11. **Insufficient Margin Rejection:** Rejects retained margins under 1.00 USDC.
12. **Fractional Quorum Rejection:** Rejects non-integer values (e.g. `2.5`).
13. **Malformed Quorum Graceful Rejection:** Safely handles `"invalid"`, `None`, and empty inputs without unhandled exceptions.
14. **Sub-Threshold Quorum Rejection:** Rejects quorum < 2.
15. **Policy Hash Mismatch Rejection:** Enforces canonical policy hash constant.
16. **String Booleans Rejection:** Rejects `"false"` or `"true"` strings for boolean fields.
17. **All-Zero Tx Hash Rejection:** Rejects `0x000...` mock transaction hashes.
18. **Missing Settlement Receipt Rejection:** Rejects empty or missing receipts.
19. **Empty Payload Graceful Rejection:** Empty or non-dict payloads fail cleanly across all 4 controls.
