# Multi-stage build for the shared `hivemind` / `hivemindd` binaries.
# See docs/operations.md for run instructions.
# protoc is required: crates/hivemind/build.rs compiles gRPC stubs via tonic.

FROM rust:1.89-bookworm AS build
WORKDIR /src
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && protoc --version
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
# Shared service; set distinct HIVEMIND_TOKEN, HIVEMIND_OAUTH_CLIENT_SECRET,
# and HIVEMIND_SIGNING_KEY at runtime (see SECURITY.md).
ENTRYPOINT ["hivemind"]
CMD ["serve", "--listen", "0.0.0.0:8787", "--root", "/var/lib/hivemind"]
