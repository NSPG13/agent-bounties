ARG SP1_SOURCE_COMMIT
ARG SP1_CIRCUIT_VERSION

FROM golang:1.26@sha256:26326682769ca980f8f1d3b1f52be2dd1c1d25270e3de3fe0c97d6bb65df3556 AS go-builder

FROM rust:1.91-slim-bookworm@sha256:8514999d4786ef12efe89239e86b3d0a021b94b9d35108c8efe6c79ca7dc1a65 AS rust-builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends clang=1:14.0-55.7~deb12u1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=go-builder /usr/local/go /usr/local/go
ENV PATH="/usr/local/go/bin:$PATH"
ENV RUSTUP_TOOLCHAIN="1.91.1"

WORKDIR /sp1
COPY . /sp1

WORKDIR /sp1/crates/recursion/gnark-cli
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/sp1/target \
    cargo build --release --locked \
    && cp ../../../target/release/sp1-recursion-gnark-cli /gnark-cli

FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241

ARG SP1_SOURCE_COMMIT
ARG SP1_CIRCUIT_VERSION

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates=20250419~deb12u1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=rust-builder /gnark-cli /gnark-cli

LABEL org.opencontainers.image.source="https://github.com/NSPG13/sp1" \
      org.opencontainers.image.revision="${SP1_SOURCE_COMMIT}" \
      app.agent-bounties.circuit-version="${SP1_CIRCUIT_VERSION}"

ENTRYPOINT ["/gnark-cli"]
