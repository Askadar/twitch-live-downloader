FROM rust:1.95.0-trixie AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=app-target,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=app-target,target=/app/target \
    cargo build --release && \
    cp /app/target/release/ld /app/ld

FROM debian:trixie-slim AS runtime

COPY --from=ghcr.io/astral-sh/uv:0.11.8 /uv /bin/uv

ENV UV_TOOL_BIN_DIR=/usr/local/bin

RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg=7:7.1.3-0+deb13u1 \
    && rm -rf /var/lib/apt/lists/*

RUN uv tool install streamlink==8.3.0 \
    && uv tool install yt-dlp[default]==2026.3.17 \
    && rm /bin/uv

COPY --from=builder /app/ld /usr/local/bin/ld

WORKDIR /app

VOLUME ["/downloads", "/chat"]

ENTRYPOINT ["ld"]
