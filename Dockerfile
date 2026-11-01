# syntax=docker/dockerfile:1
# Reproducible: base images pinned by digest, Rust deps by Cargo.lock (--locked), Webamp by web/package-lock.json.

FROM node:24.11-alpine@sha256:682368d8253e0c3364b803956085c456a612d738bd635926d73fa24db3ce53d7 AS web
WORKDIR /src
COPY scripts/vendor-web.sh scripts/
COPY web/package.json web/package-lock.json web/
RUN sh scripts/vendor-web.sh

FROM rust:1.99-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 AS build
WORKDIR /src
COPY rust-toolchain.toml .
RUN rustup toolchain install
COPY . .
COPY --from=web /src/web/vendor web/vendor
# lull's own Last.fm app key is compiled in when the build passes it (core::config), so a listener
# only clicks Connect. Optional: a build without these secrets just has no built-in key.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=secret,id=lastfm_key,env=LULL_LASTFM_API_KEY \
    --mount=type=secret,id=lastfm_secret,env=LULL_LASTFM_API_SECRET \
    cargo build --release --locked --bin lull && cp target/release/lull /lull

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e
COPY --from=build /lull /usr/local/bin/lull
ENV LULL_LISTEN=0.0.0.0:8080 LULL_DATA_DIR=/data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD ["lull", "healthcheck"]
ENTRYPOINT ["lull"]
