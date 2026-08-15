#!/usr/bin/env bash
set -euo pipefail

: "${SP1_GNARK_IMAGE:?SP1_GNARK_IMAGE must name the source-built safe image}"

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
