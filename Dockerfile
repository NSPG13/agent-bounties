# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.88
FROM rust:${RUST_VERSION}-bookworm AS builder

WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY contracts ./contracts
COPY migrations ./migrations
COPY schemas ./schemas

ARG APP_PACKAGE=api
ARG APP_BINARY=api
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/workspace/target,sharing=locked \
    cargo build --locked --release -p "${APP_PACKAGE}" \
    && cp "target/release/${APP_BINARY}" /tmp/agent-bounties-service

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 10001 app
COPY --from=builder /tmp/agent-bounties-service /usr/local/bin/agent-bounties-service

USER app
ENV RUST_LOG=info
EXPOSE 8080 8090

ENTRYPOINT ["/usr/local/bin/agent-bounties-service"]
