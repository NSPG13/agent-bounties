# Open Competition V2 Groth16 Ceremony

This pinned Go tool runs the gnark BN254 Groth16 Phase 1 and circuit-specific
Phase 2 MPC used by Open Competition V2 Beta2. Each command emits one JSON
record containing exact input and output hashes.

Run each `contribute-*` command in an isolated worker with no network and a
fresh output path. Keep every ordered contribution. After the last
contribution, use a public 32-byte-or-longer beacon that did not exist when the
last contributor ran. `verify-phase1` and `finalize` verify the complete chains
before producing the final PK, VK, and Solidity verifier.

Never use `groth16.Setup` output as a mainnet release key. Never reuse a
contributor environment or claim independent participation when one operator
controlled every entropy source.

The release orchestrator runs every contribution in a fresh, networkless,
read-only container and fetches a new drand beacon only after each phase's last
contribution. Build the runtime from `ops/open-competition-v2-ceremony.Dockerfile`
and run `scripts/run_open_competition_v2_groth16_ceremony.sh`. The initial Beta2
ceremony is internally orchestrated and must not be described as independently
contributed.
