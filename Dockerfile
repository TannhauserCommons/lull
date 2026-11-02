# syntax=docker/dockerfile:1
# Reproducible: base images pinned by digest, Rust deps by Cargo.lock (--locked), Webamp by web/package-lock.json.

# The web UI as the binary embeds it: Webamp vendored, then every file minified and pre-compressed
# by scripts/web-dist (pinned esbuild and lightningcss; docs/context/quality.md).
FROM node:24.11-alpine@sha256:682368d8253e0c3364b803956085c456a612d738bd635926d73fa24db3ce53d7 AS web
WORKDIR /src
COPY scripts/vendor-web.sh scripts/
COPY web/package.json web/package-lock.json web/
RUN sh scripts/vendor-web.sh
COPY scripts/web-dist/package.json scripts/web-dist/package-lock.json scripts/web-dist/
RUN npm ci --prefix scripts/web-dist --ignore-scripts --no-audit --no-fund
COPY scripts/web-dist/build.mjs scripts/web-dist/
COPY web web
RUN node scripts/web-dist/build.mjs web dist

FROM rust:1.99-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 AS build
WORKDIR /src
COPY rust-toolchain.toml .
RUN rustup toolchain install
COPY . .
COPY --from=web /src/dist web
# lull's own Last.fm app key is compiled in when the build passes it (core::config), so a listener
# only clicks Connect. Optional: a build without these secrets just has no built-in key.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=secret,id=lastfm_key,env=LULL_LASTFM_API_KEY \
    --mount=type=secret,id=lastfm_secret,env=LULL_LASTFM_API_SECRET \
    cargo build --release --locked --bin lull && cp target/release/lull /lull

# YouTube (docs/context/youtube.md): the standalone yt-dlp, which needs no Python, and Deno, the
# JavaScript runtime yt-dlp needs for YouTube. Both pinned by version and SHA-256. The hub may
# fetch a newer yt-dlp into /data (Keep yt-dlp up to date); this one stays the fallback.
FROM node:24.11-alpine@sha256:682368d8253e0c3364b803956085c456a612d738bd635926d73fa24db3ce53d7 AS youtube
ARG YTDLP_VERSION=2026.10.26
ARG DENO_VERSION=2.9.8
ADD --chmod=755 --checksum=sha256:767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795 \
    https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp_linux /out/yt-dlp
ADD --checksum=sha256:e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609 \
    https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/deno-x86_64-unknown-linux-gnu.zip /tmp/deno.zip
RUN unzip -q /tmp/deno.zip -d /out && chmod 755 /out/deno

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e
COPY --from=build /lull /usr/local/bin/lull
COPY --from=youtube /out/yt-dlp /out/deno /usr/local/bin/
# The standalone yt-dlp links zlib, which distroless cc leaves out; the build stage is the same
# Debian release.
COPY --from=build /usr/lib/x86_64-linux-gnu/libz.so.1 /usr/lib/x86_64-linux-gnu/libz.so.1
# Both must run here, or the image isn't built (exec form: distroless has no shell). yt-dlp unpacks
# itself into a temporary folder; at run time the hub points it at /data.
RUN --mount=type=tmpfs,target=/tmp ["/usr/local/bin/yt-dlp", "--version"]
RUN --mount=type=tmpfs,target=/tmp ["/usr/local/bin/deno", "--version"]
ENV LULL_LISTEN=0.0.0.0:8080 LULL_DATA_DIR=/data \
    LULL_YTDLP=/usr/local/bin/yt-dlp LULL_DENO=/usr/local/bin/deno
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD ["lull", "healthcheck"]
ENTRYPOINT ["lull"]
