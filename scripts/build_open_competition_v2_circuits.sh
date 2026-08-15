#!/usr/bin/env bash
set -euo pipefail

: "${SP1_GNARK_IMAGE:?SP1_GNARK_IMAGE must name the source-built safe image}"

if [[ -r /proc/meminfo ]]; then
  memory_kib="$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)"
  swap_kib="$(awk '/^SwapTotal:/ { print $2 }' /proc/meminfo)"
  minimum_memory_kib=$((180 * 1024 * 1024))
  minimum_combined_kib=$((288 * 1024 * 1024))
  if (( memory_kib < minimum_memory_kib )); then
    echo "circuit build requires at least 180 GiB physical memory" >&2
    exit 2
  fi
  if (( memory_kib + swap_kib < minimum_combined_kib )); then
    echo "circuit build requires at least 288 GiB combined memory and swap" >&2
    exit 2
  fi
fi

source_root="${1:-.sp1-safe}"
source_root="$(cd "$source_root" && pwd)"
build_dir="$source_root/crates/prover/build"

rm -rf "$build_dir"
mkdir -p "$build_dir/groth16" "$build_dir/plonk"

cd "$source_root"
RUST_LOG=debug cargo run -p sp1-prover --release --bin build_groth16_bn254 -- \
  --build-dir="$build_dir/groth16"
RUST_LOG=debug cargo run -p sp1-prover --release --bin build_plonk_bn254 -- \
  --build-dir="$build_dir/plonk"
