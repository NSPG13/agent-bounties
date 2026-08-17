#!/usr/bin/env bash
set -euo pipefail

: "${OPEN_COMPETITION_V2_CEREMONY_IMAGE:?set the digest-pinned ceremony image}"
: "${OPEN_COMPETITION_V2_R1CS_SHA256:?set the pinned R1CS hash}"
: "${OPEN_COMPETITION_V2_PHASE1_1_SHA256:?set the pinned first contribution hash}"
: "${OPEN_COMPETITION_V2_PHASE1_2_SHA256:?set the pinned second contribution hash}"
: "${OPEN_COMPETITION_V2_PHASE1_COMMONS_SHA256:?set the pinned commons hash}"
: "${OPEN_COMPETITION_V2_PHASE1_BEACON:?set the pinned Phase 1 beacon}"

if [[ $# -ne 4 ]]; then
  echo "usage: $0 OUTPUT_DIR R1CS SAFE_REFERENCE_WRAPPER REPOSITORY" >&2
  exit 2
fi
output_dir="$(cd "$1" && pwd)"
r1cs="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
reference="$(cd "$(dirname "$3")" && pwd)/$(basename "$3")"
repo="$(cd "$4" && pwd)"

python3 "$repo/scripts/verify_open_competition_v2_groth16_phase1_checkpoint.py" \
  --root "$output_dir" --r1cs "$r1cs" \
  --r1cs-sha256 "$OPEN_COMPETITION_V2_R1CS_SHA256" \
  --phase1-1-sha256 "$OPEN_COMPETITION_V2_PHASE1_1_SHA256" \
  --phase1-2-sha256 "$OPEN_COMPETITION_V2_PHASE1_2_SHA256" \
  --commons-sha256 "$OPEN_COMPETITION_V2_PHASE1_COMMONS_SHA256" \
  --beacon "$OPEN_COMPETITION_V2_PHASE1_BEACON"

container() {
  docker run --rm --network none --read-only \
    --user "$(id -u):$(id -g)" \
    -v "$output_dir:/data" \
    -v "$r1cs:/inputs/groth16_circuit.bin:ro" \
    "$OPEN_COMPETITION_V2_CEREMONY_IMAGE" "$@"
}
fetch_beacon() {
  local destination="$1"
  local baseline="$destination.baseline"
  local candidate="$destination.candidate"
  curl --fail --silent --show-error --proto '=https' --tlsv1.2 \
    https://api.drand.sh/public/latest -o "$baseline"
  local next_round
  next_round="$(python3 - "$baseline" <<'PY'
import json, re, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
randomness = value.get("randomness", "")
round_number = value.get("round")
if not isinstance(round_number, int) or round_number <= 0 or not re.fullmatch(r"[0-9a-f]{64}", randomness):
    raise SystemExit("drand response lacks canonical 32-byte randomness")
print(round_number + 1)
PY
  )"
  for _ in $(seq 1 120); do
    if curl --fail --silent --show-error --proto '=https' --tlsv1.2 \
      "https://api.drand.sh/public/$next_round" -o "$candidate" 2>/dev/null; then
      python3 - "$candidate" "$next_round" <<'PY'
import json, re, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
if value.get("round") != int(sys.argv[2]) or not re.fullmatch(r"[0-9a-f]{64}", value.get("randomness", "")):
    raise SystemExit("drand future-round response is invalid")
PY
      mv "$candidate" "$destination"
      rm -f "$baseline"
      python3 - "$destination" <<'PY'
import json, sys
print("0x" + json.load(open(sys.argv[1], encoding="utf-8"))["randomness"])
PY
      return 0
    fi
    sleep 2
  done
  rm -f "$baseline" "$candidate"
  echo "timed out waiting for a post-contribution drand round" >&2
  return 1
}

rm -f "$output_dir"/phase2-*.bin "$output_dir"/0{6,7,8}-phase2-*.json \
  "$output_dir/phase2-beacon.json" "$output_dir/10-finalize.json" \
  "$output_dir/groth16_pk.bin" "$output_dir/groth16_vk.bin" \
  "$output_dir/Groth16Verifier.sol" "$output_dir/SP1VerifierGroth16.sol" \
  "$output_dir/transcript.json" "$output_dir/verification-evidence.json" \
  "$output_dir/verifier-hash.txt" "$output_dir/phase2-finalize.ready" \
  "$output_dir/complete"

coordinator_pid=""
stop_coordinator() {
  if [[ -n "$coordinator_pid" ]] && kill -0 "$coordinator_pid" 2>/dev/null; then
    kill "$coordinator_pid" 2>/dev/null || true
    wait "$coordinator_pid" 2>/dev/null || true
  fi
}
trap stop_coordinator EXIT
container coordinate-phase2 --r1cs /inputs/groth16_circuit.bin \
  --commons /data/phase1-commons.bin --initial /data/phase2-init.bin \
  --initial-record /data/06-phase2-init.json \
  --inputs /data/phase2-1.bin,/data/phase2-2.bin \
  --beacon-file /data/phase2-beacon.json --ready-file /data/phase2-finalize.ready \
  --pk /data/groth16_pk.bin --vk /data/groth16_vk.bin \
  --solidity /data/Groth16Verifier.sol --timeout 48h \
  > "$output_dir/10-finalize.json" &
coordinator_pid="$!"
while [[ ! -s "$output_dir/06-phase2-init.json" ]]; do
  if ! kill -0 "$coordinator_pid" 2>/dev/null; then
    wait "$coordinator_pid"
    exit 1
  fi
  sleep 5
done
for id in 1 2; do
  previous=phase2-init.bin
  if (( id > 1 )); then previous="phase2-$((id - 1)).bin"; fi
  container contribute-phase2 --input "/data/$previous" \
    --output "/data/phase2-$id.bin" --contribution-id "$id" \
    | tee "$output_dir/0$((id + 6))-phase2-$id.json"
done
fetch_beacon "$output_dir/phase2-beacon.json" >/dev/null
touch "$output_dir/phase2-finalize.ready"
wait "$coordinator_pid"
coordinator_pid=""
trap - EXIT

cd "$repo"
python3 scripts/rebind_open_competition_v2_sp1_wrapper.py \
  --reference "$reference" --verifying-key "$output_dir/groth16_vk.bin" \
  --proof-system groth16 --output "$output_dir/SP1VerifierGroth16.sol" \
  | tee "$output_dir/verifier-hash.txt"
python3 - "$output_dir" "$r1cs" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
records = [json.loads(path.read_text(encoding="utf-8")) for path in sorted(root.glob("[0-9][0-9]-*.json"))]
value = {
    "schema_version": "agent-bounties/open-competition-v2-beta3-groth16-mpc-transcript-v1",
    "r1cs": pathlib.Path(sys.argv[2]).name,
    "records": records,
    "phase1_beacon": json.loads((root / "phase1-beacon.json").read_text()),
    "phase2_beacon": json.loads((root / "phase2-beacon.json").read_text()),
}
(root / "transcript.json").write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
PY
python3 scripts/verify_open_competition_v2_groth16_ceremony.py \
  --root "$output_dir" --r1cs "$r1cs" \
  --ceremony-uri https://github.com/NSPG13/agent-bounties/issues/888 \
  --output "$output_dir/verification-evidence.json"
find "$output_dir" -maxdepth 1 -type f ! -name SHA256SUMS -print0 \
  | sort -z | xargs -0 sha256sum > "$output_dir/SHA256SUMS"
touch "$output_dir/complete"
