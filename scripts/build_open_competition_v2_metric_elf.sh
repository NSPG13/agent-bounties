#!/usr/bin/env bash
set -euo pipefail

profile="${1:?usage: build_open_competition_v2_metric_elf.sh PROFILE OUTPUT_DIRECTORY}"
output_directory="${2:?usage: build_open_competition_v2_metric_elf.sh PROFILE OUTPUT_DIRECTORY}"
repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
program="$repository/programs/$profile/program"

case "$profile" in
  public-vector-metric-v1|structured-artifact-metric-v1) ;;
  *) echo "unsupported metric profile: $profile" >&2; exit 2 ;;
esac

test -f "$program/Cargo.lock"
mkdir -p "$output_directory"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rustflags="--remap-path-prefix=$repository=/agent-bounties,--remap-path-prefix=$cargo_home=/cargo-home,--remap-path-prefix=$HOME/.sp1=/sp1-home"

(
  cd "$program"
  cargo prove build --locked --rustflags="$rustflags" --output-directory "$output_directory"
)

elf="$output_directory/$profile-program"
test -s "$elf"
toolchain="$(rustc +succinct --print sysroot)"
host="$(rustc +succinct -vV | sed -n 's/^host: //p')"
strip="$toolchain/lib/rustlib/$host/bin/llvm-strip"
test -x "$strip"
"$strip" --strip-all "$elf"
test -s "$elf"
printf '%s\n' "$elf"
