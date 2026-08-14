# Beta2 verifier assets

`open-competition-v2-beta2-verifier-assets.json` is a generated release artifact,
not a hand-written configuration file. It is absent until the pinned CPU prover:

1. builds the patched Groth16 and PLONK verifier contracts;
2. generates and self-verifies one Groth16 and two PLONK proofs; and
3. runs `scripts/build_open_competition_v2_verifier_assets.py` against those exact
   artifacts and proof records.

Deployment planning fails closed while the JSON file is absent, malformed, or
inconsistent with its code hashes. Never create placeholder verifier assets.
