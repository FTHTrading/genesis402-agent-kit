# LegacyChain node + CLI.
#   docker build -t legacychain ./legacychain
# Secrets (key file, LC_PASSPHRASE, LC_STORAGE_KEY, facilitator auth) are
# mounted or passed at run time; nothing secret is baked into the image.
FROM rust:1.90-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked --bin legacychain-node --bin lc

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /var/lib/legacychain legacychain
COPY --from=build /src/target/release/legacychain-node /src/target/release/lc /usr/local/bin/
USER legacychain
WORKDIR /var/lib/legacychain
EXPOSE 7402
HEALTHCHECK --interval=15s --timeout=3s CMD curl -sf http://127.0.0.1:7402/v1/health || exit 1
ENTRYPOINT ["legacychain-node"]
CMD ["--listen", "0.0.0.0:7402", "--data", "/var/lib/legacychain/data"]
