#!/usr/bin/env bash
set -euo pipefail

: "${OPEN_COMPETITION_V2_CEREMONY_IMAGE:?set the digest-pinned ceremony image}"

if [[ $# -ne 3 ]]; then
  echo "usage: $0 OUTPUT_DIR R1CS SAFE_V5_REFERENCE_WRAPPER" >&2
  exit 2
fi

output_dir="$(mkdir -p "$1" && cd "$1" && pwd)"
r1cs="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
reference="$(cd "$(dirname "$3")" && pwd)/$(basename "$3")"
test -f "$r1cs"
test -f "$reference"
if find "$output_dir" -mindepth 1 -print -quit | grep -q .; then
  echo "ceremony output directory must be empty: $output_dir" >&2
  exit 2
fi
cp --reflink=auto "$r1cs" "$output_dir/groth16_circuit.bin"

container() {
  docker run --rm --network none --read-only \
    -v "$output_dir:/data" \
    -v "$r1cs:/inputs/groth16_circuit.bin:ro" \
    "$OPEN_COMPETITION_V2_CEREMONY_IMAGE" "$@"
}

fetch_beacon() {
  local destination="$1"
  curl --fail --silent --show-error --proto '=https' --tlsv1.2 \
    https://api.drand.sh/public/latest -o "$destination"
  python - "$destination" <<'PY'
import json, re, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
randomness = value.get("randomness", "")
if not re.fullmatch(r"[0-9a-f]{64}", randomness):
    raise SystemExit("drand response lacks canonical 32-byte randomness")
print("0x" + randomness)
PY
}

container init-phase1 --r1cs /inputs/groth16_circuit.bin --output /data/phase1-init.bin \
  | tee "$output_dir/01-phase1-init.json"
for id in 1 2; do
  previous="phase1-init.bin"
  if (( id > 1 )); then previous="phase1-$((id - 1)).bin"; fi
  container contribute-phase1 --input "/data/$previous" --output "/data/phase1-$id.bin" \
    --contribution-id "$id" | tee "$output_dir/0$((id + 1))-phase1-$id.json"
done
phase1_beacon="$(fetch_beacon "$output_dir/phase1-beacon.json")"
container verify-phase1 --r1cs /inputs/groth16_circuit.bin \
  --inputs /data/phase1-1.bin,/data/phase1-2.bin \
  --beacon "$phase1_beacon" --output /data/phase1-commons.bin \
  | tee "$output_dir/05-phase1-verify.json"

container init-phase2 --r1cs /inputs/groth16_circuit.bin \
  --commons /data/phase1-commons.bin --output /data/phase2-init.bin \
  | tee "$output_dir/06-phase2-init.json"
for id in 1 2; do
  previous="phase2-init.bin"
  if (( id > 1 )); then previous="phase2-$((id - 1)).bin"; fi
  container contribute-phase2 --input "/data/$previous" --output "/data/phase2-$id.bin" \
    --contribution-id "$id" | tee "$output_dir/0$((id + 6))-phase2-$id.json"
done
phase2_beacon="$(fetch_beacon "$output_dir/phase2-beacon.json")"
container finalize --r1cs /inputs/groth16_circuit.bin --commons /data/phase1-commons.bin \
  --inputs /data/phase2-1.bin,/data/phase2-2.bin \
  --beacon "$phase2_beacon" --pk /data/groth16_pk.bin --vk /data/groth16_vk.bin \
  --solidity /data/Groth16Verifier.sol | tee "$output_dir/10-finalize.json"

python scripts/rebind_open_competition_v2_sp1_wrapper.py \
  --reference "$reference" --verifying-key "$output_dir/groth16_vk.bin" \
  --proof-system groth16 --output "$output_dir/SP1VerifierGroth16.sol" \
  | tee "$output_dir/verifier-hash.txt"
python - "$output_dir" "$r1cs" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
records = []
for path in sorted(root.glob("[0-9][0-9]-*.json")):
    records.append(json.loads(path.read_text(encoding="utf-8")))
value = {
    "schema_version": "agent-bounties/open-competition-v2-beta3-groth16-mpc-transcript-v1",
    "r1cs": pathlib.Path(sys.argv[2]).name,
    "records": records,
    "phase1_beacon": json.loads((root / "phase1-beacon.json").read_text()),
    "phase2_beacon": json.loads((root / "phase2-beacon.json").read_text()),
}
(root / "transcript.json").write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
PY
python scripts/verify_open_competition_v2_groth16_ceremony.py \
  --root "$output_dir" --r1cs "$r1cs" \
  --ceremony-uri https://github.com/NSPG13/agent-bounties/issues/888 \
  --output "$output_dir/verification-evidence.json"
touch "$output_dir/complete"
