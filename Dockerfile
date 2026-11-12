# Multi-stage build for the shared `hivemind` / `hivemindd` binaries.
# See docs/operations.md for run instructions.

FROM rust:1.89-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p hivemind \
    && strip target/release/hivemind target/release/hivemindd

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/hivemind /usr/local/bin/hivemind
COPY --from=build /src/target/release/hivemindd /usr/local/bin/hivemindd
RUN useradd --system --create-home --home-dir /var/lib/hivemind --shell /usr/sbin/nologin hivemind \
    && mkdir -p /var/lib/hivemind \
    && chown -R hivemind:hivemind /var/lib/hivemind
USER hivemind
ENV HIVEMIND_ROOT=/var/lib/hivemind \
    HIVEMIND_DIR=/var/lib/hivemind/store \
    RUST_LOG=info
EXPOSE 8787 8788
VOLUME ["/var/lib/hivemind"]
# Shared service; set HIVEMIND_TOKEN at runtime.
ENTRYPOINT ["hivemind"]
CMD ["serve", "--listen", "0.0.0.0:8787", "--root", "/var/lib/hivemind"]
