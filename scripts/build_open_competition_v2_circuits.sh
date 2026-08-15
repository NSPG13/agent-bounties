#!/usr/bin/env bash
set -euo pipefail

: "${SP1_GNARK_IMAGE:?SP1_GNARK_IMAGE must name the source-built safe image}"

if [[ -r /proc/meminfo ]]; then
  memory_kib="$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)"
  minimum_memory_kib=$((250 * 1024 * 1024))
  if (( memory_kib < minimum_memory_kib )); then
    echo "circuit build requires a 256 GiB host (at least 250 GiB reported physical memory); swap does not qualify" >&2
    exit 2
  fi
fi

source_root="${1:-.sp1-safe}"
source_root="$(cd "$source_root" && pwd)"
build_dir="$source_root/crates/prover/build"
available_disk_kib="$(df -Pk "$source_root" | awk 'NR == 2 { print $4 }')"
minimum_disk_kib=$((60 * 1024 * 1024))
if (( available_disk_kib < minimum_disk_kib )); then
  echo "circuit build requires at least 60 GiB free disk" >&2
  exit 2
fi

rm -rf "$build_dir"
mkdir -p "$build_dir/groth16" "$build_dir/plonk"

cd "$source_root"
RUST_LOG=debug cargo run -p sp1-prover --release --bin build_groth16_bn254 -- \
  --build-dir="$build_dir/groth16"
RUST_LOG=debug cargo run -p sp1-prover --release --bin build_plonk_bn254 -- \
  --build-dir="$build_dir/plonk"
