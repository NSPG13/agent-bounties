#!/usr/bin/env python3
"""Verify the Beta3 gnark image build is local, source-bound and digest-pinned."""

from __future__ import annotations

import json
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
DOCKERFILE = ROOT / "ops/open-competition-v2-gnark-safe.Dockerfile"
WORKFLOW = ROOT / ".github/workflows/open-competition-v2-beta3-release.yml"
CIRCUIT_BUILDER = ROOT / "scripts/build_open_competition_v2_circuits.sh"
EXPECTED_BASES = (
    "golang:1.26@sha256:26326682769ca980f8f1d3b1f52be2dd1c1d25270e3de3fe0c97d6bb65df3556",
    "rust:1.91-slim-bookworm@sha256:8514999d4786ef12efe89239e86b3d0a021b94b9d35108c8efe6c79ca7dc1a65",
    "debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241",
)


def verify(
    dockerfile: Path = DOCKERFILE,
    workflow: Path = WORKFLOW,
    circuit_builder: Path = CIRCUIT_BUILDER,
) -> dict[str, object]:
    image_source = dockerfile.read_text(encoding="utf-8")
    release_source = workflow.read_text(encoding="utf-8")
    circuit_source = circuit_builder.read_text(encoding="utf-8")
    from_images = tuple(
        match.group(1)
        for match in re.finditer(r"^FROM\s+(\S+)", image_source, re.MULTILINE)
    )
    if from_images != EXPECTED_BASES:
        raise ValueError("gnark image bases must match the three exact digests")
    required_image_fragments = (
        'ENV RUSTUP_TOOLCHAIN="1.91.1"',
        "cargo build --release --locked",
        'org.opencontainers.image.revision="${SP1_SOURCE_COMMIT}"',
        'app.agent-bounties.circuit-version="${SP1_CIRCUIT_VERSION}"',
    )
    if any(fragment not in image_source for fragment in required_image_fragments):
        raise ValueError("gnark image omits a compiler or source-identity pin")
    required_workflow_fragments = (
        "SP1_GNARK_IMAGE: agent-bounties-sp1-gnark-safe-v5:f6a2dffc42c322d0a6d8f5b5ae06fb76986ae12d",
        "SP1_GNARK_RUNTIME_IMAGE: ghcr.io/succinctlabs/sp1-gnark:agent-bounties-sp1-safe-v5",
        "OPEN_COMPETITION_V2_TRUSTED_SETUP_ROOT: /mnt/agent-bounties-artifacts/sp1-safe-v5-trusted",
        "rm -rf .sp1-safe/crates/prover/build",
        "--file -",
        "< ops/open-competition-v2-gnark-safe.Dockerfile",
        'docker image inspect "$SP1_GNARK_IMAGE"',
        'sha256sum "$SP1_GNARK_IMAGE" /gnark-cli',
        'docker tag "$SP1_GNARK_IMAGE" "$SP1_GNARK_RUNTIME_IMAGE"',
        'docker image inspect "$SP1_GNARK_RUNTIME_IMAGE"',
        "target/gnark-cli.sha256",
        'test -f "$trusted_root/trusted-setup.json"',
        'cp -a "$trusted_root/$system" ".sp1-safe/crates/prover/build/$system"',
        '--trusted-setup-manifest "$OPEN_COMPETITION_V2_TRUSTED_SETUP_ROOT/trusted-setup.json"',
    )
    if any(fragment not in release_source for fragment in required_workflow_fragments):
        raise ValueError(
            "release workflow does not build the local gnark image and consume verified setup assets"
        )
    if "bash scripts/build_open_competition_v2_circuits.sh .sp1-safe" in release_source:
        raise ValueError("release workflow reintroduces the unsafe single-party setup route")
    if 'docker pull "$SP1_GNARK_RUNTIME_IMAGE"' in release_source or 'docker push "$SP1_GNARK_RUNTIME_IMAGE"' in release_source:
        raise ValueError("release workflow must not pull or publish the runtime compatibility alias")
    required_builder_fragments = (
        "minimum_memory_kib=$((250 * 1024 * 1024))",
        "minimum_disk_kib=$((60 * 1024 * 1024))",
        "swap does not qualify",
        'source_root="$(cd "$source_root" && pwd)"',
        '--build-dir="$build_dir/groth16"',
        '--build-dir="$build_dir/plonk"',
    )
    if any(fragment not in circuit_source for fragment in required_builder_fragments):
        raise ValueError("circuit builder omits a capacity or absolute-path gate")
    return {
        "schema": "agent-bounties/open-competition-v2-gnark-image-check-v1",
        "status": "digest_pinned_local_build_with_trusted_setup",
        "base_images": list(EXPECTED_BASES),
        "dockerfile": str(dockerfile.relative_to(ROOT)).replace("\\", "/"),
    }


if __name__ == "__main__":
    print(json.dumps(verify(), sort_keys=True))
