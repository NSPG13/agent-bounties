# Canary claim-next-action proof

Contract: 
Solver: 
Bounty issue: #664

## Source snapshot (repo root)

- **Source commit:** 
- **Source root:** repository root (no subdirectory filter)
- **source_snapshot_digest:** 
- **file_count:** 658
- **total_bytes:** 11164477
- **Directory digest domain:**  (Rust )
- **Archive method:** GitHub codeload tar.gz of commit (same as sandboxed regression verifier)

## Artifact

- **Path:** 
- **artifact_digest:** 
- **Implementation commit:** 

## Benchmark

direct_agent_loop_benchmark=passed task=claim-next-action

## Script (committed at source commit)



## Notes for reviewers

Previous proof incorrectly used a subdirectory-only digest over .
This revision hashes the **full repository root** at  with the canonical directory digest algorithm so  matches sandboxed regression staging of the repo root.
