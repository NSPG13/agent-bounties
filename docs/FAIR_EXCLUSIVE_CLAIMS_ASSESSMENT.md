# Fair Exclusive Claims & Proportional Bonds Assessment

## 1. Executive Summary & Governing Protocol
This document defines the formal protocol assessment for **Fair Exclusive Claims and Proportional Solver Bonds** under Issue #794. It establishes the governing invariants required to maintain fair opportunity distribution, prevent claim hoarding, ensure rejection solvency, and maintain additive backward compatibility across all on-chain bounty contracts.

## 2. Core Protocol Invariants (#794 Compliance)

### Invariant 1: One Active Slot Invariant (`one_active_slot_invariant`)
- **Specification**: The protocol enforces a strict invariant of at most **1 active exclusive claim slot per solver address** across all canonical fair-claim bounties. A solver cannot reserve a second exclusive claim until their current active claim is either settled, expired, or voluntarily released.

### Invariant 2: Hour-Scale Renewal Evidence (`hour_scale_renewal_evidence`)
- **Specification**: Initial reservation windows use **hour-scale durations** (e.g., 24–72 hours) and can only be renewed by submitting cryptographic, public, content-addressed **progress evidence URIs** (IPFS/Arweave hashes) demonstrating measurable work completed before the window expires.

### Invariant 3: Bond / Rejection Solvency (`bond_rejection_solvency`)
- **Specification**: Solver reservation bonds scale proportionally with bounty reward magnitude and reservation duration. The protocol maintains **100% rejection solvency**, ensuring that slashed bonds cover verification overhead and rejected claims never dilute the core protocol treasury or bounty reward pools.

### Invariant 4: Precommitted Symmetric Appeals (`precommitted_symmetric_appeals`)
- **Specification**: Any non-deterministic verifier or reviewer path requires **precommitted symmetric appeal contracts** established prior to bounty funding. Both solver and poster stake equal collateral subject to deterministic arbitration, eliminating unilateral maintainer rejection bias.

### Invariant 5: Historical Bytecode & Payment Event Compatibility (`historical_bytecode_compatibility`)
- **Specification**: The Fair Exclusive Claims registry operates as an **additive protocol layer** that preserves complete backward compatibility with all historical V1/V2/V3/V4 on-chain bytecode. Payment settlement strictly emits canonical `BountySettled` events as proof of solver payment.

## 3. Reference Protocol Contracts & Verification
- Governing Spec: `docs/FAIR_EXCLUSIVE_CLAIMS_ASSESSMENT.md`
- Protocol Event: `BountySettled`
- Validator script: `scripts/validate-fair-claims.py`
