# Evidence Shape & Policy Validator Helper: Issue #648 (MCP Interoperability Task)

**Target Issue:** `NSPG13/agent-bounties#648`  
**Module:** `solutions/issue-648`  
**Role:** Non-Authoritative Client-Side Policy & Shape Validator Helper  

---

> [!NOTE]
> **Boundary of Authority:** This module is an offline client-side validator helper for syntactic payload-shape, strict typing, and local policy sanity checks. It does **not** constitute canonical on-chain verification, substitute for parent/child coordination events, or authorize registration, bonds, child funding, or bounty settlement.

---

## 📋 Evaluated Policy Controls

| Control ID | Description | Rule Conformance & Sanitization |
| :--- | :--- | :--- |
| `CONTROL_01_PARTICIPANT_INDEPENDENCE` | Verifies parent solver and child solver are independent participants. | Distinct valid Base EVM hex addresses (`0x...`). Rejects identical participants and all-zero (`0x000...`) addresses. |
| `CONTROL_02_MARGIN_AND_FUNDING` | Validates strict numeric funding target and retained gross margin. | Strictly rejects booleans, strings, `NaN`, and infinities. Enforces `child_target >= 1.00 USDC` and `(parent_reward - child_target) >= 1.00 USDC` within bounded limits. |
| `CONTROL_03_VERIFIER_QUORUM` | Validates strict integer quorum threshold, framework, and immutable policy hash. | Framework must equal `sandboxed_regression_v1` with `quorum >= 2` (strict integer, rejecting floats like `2.5`, strings, and null). Router and immutable policy hash must match canonical constants. |
| `CONTROL_04_LANE_READINESS_AND_SETTLEMENT` | Validates profile approval, non-zero tx hash, and receipt structures. | Requires strict boolean `child_profile_approved == True` (rejecting string `"false"`). Requires canonical `BountySettled` event, strict boolean `settled == True`, non-zero `tx_hash`, valid recipient, and child contract. |

---

## 🧪 Unit Test Suite Execution

The accompanying unit test suite (`test_solution.py`) contains 14 deterministic tests covering valid conformance and negative control failures:

```bash
python3 -m unittest test_solution.py -v
```

### Assertions Covered (14 Tests):
1. **Full Conformance:** Deterministic passage of all 4 controls on valid evidence.
2. **Identical Participants Rejection:** Rejects parent == child.
3. **All-Zero Address Rejection:** Rejects `0x0000000000000000000000000000000000000000`.
4. **String & Boolean Amounts Rejection:** Rejects `True`/`False` or strings as monetary values.
5. **NaN & Infinity Rejection:** Rejects non-finite floating point amounts.
6. **Insufficient Margin Rejection:** Rejects retained margins under 1.00 USDC.
7. **Fractional Quorum Rejection:** Rejects non-integer values (e.g. `2.5`).
8. **Malformed Quorum Graceful Rejection:** Safely handles `"invalid"`, `None`, and empty inputs without unhandled exceptions.
9. **Sub-Threshold Quorum Rejection:** Rejects quorum < 2.
10. **Policy Hash Mismatch Rejection:** Enforces canonical policy hash constant.
11. **String Booleans Rejection:** Rejects `"false"` or `"true"` strings for boolean fields.
12. **All-Zero Tx Hash Rejection:** Rejects `0x000...` mock transaction hashes.
13. **Missing Settlement Receipt Rejection:** Rejects empty or missing receipts.
14. **Empty Payload Graceful Rejection:** Empty or non-dict payloads fail cleanly across all 4 controls.
