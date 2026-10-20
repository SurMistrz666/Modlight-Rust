# syntax=docker/dockerfile:1.7
#
# Modlight server (Rust port), production image.
#
#   docker build -f server/docker/Dockerfile server        # context = server/
#
# Stages:
#   web        the SPA (server/web, Vite) -> /src/web/dist
#   toolchain  Debian 12 + Rust + the tools sharp-libvips' build script needs
#   libvips    libvips 8.17.3 built by sharp-libvips 1.2.4's own build script
#              (docker/sharp-libvips/posix.sh, unmodified): the same library,
#              the same dependency versions and the same feature set as the
#              libvips-cpp.so that the reference's sharp 0.34.5 loads
#   server     the release binary, linked against that libvips-cpp.so
#   runtime    Debian 12 slim + ffmpeg 5.1 (the reference image's ffmpeg) +
#              the binary, migrations, messages, public/, web/dist
#
# The upstream image is node:24-bookworm-slim with Debian's ffmpeg; this one is
# debian bookworm-slim with the same ffmpeg package and no Node at runtime.

ARG DEBIAN_IMAGE=debian:bookworm-20261005-slim

# ── web: build the SPA ────────────────────────────────────────────────────────
FROM node:24.22.0-bookworm-slim AS web
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN --mount=type=cache,id=modlight-pnpm,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile
COPY web/ ./
COPY messages/ /src/messages/
RUN pnpm run build && test -f dist/index.html

# ── toolchain: compilers for libvips' dependencies and for the server ─────────
FROM rust:1.99.0-slim-bookworm AS toolchain
# The packages sharp-libvips installs in its own build image
# (platforms/linux-*/Dockerfile), Debian names. No -dev library packages: every
# dependency of libvips is built from source by posix.sh, and a system copy
# that a configure script could pick up instead would change the result.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      autoconf automake bzip2 ca-certificates cmake curl file g++ gcc gettext git gperf jq \
      libssl-dev libtool make patch pkg-config python3 python3-venv tar xz-utils \
 && rm -rf /var/lib/apt/lists/*
# meson and ninja from PyPI, as sharp-libvips does (Debian 12's meson is too old
# for GLib 2.86); cargo-c for librsvg.
RUN python3 -m venv /opt/meson \
 && /opt/meson/bin/pip install --no-cache-dir meson==1.9.1 ninja==1.13.0 tomli==2.2.1
ENV PATH=/opt/meson/bin:$PATH
RUN --mount=type=cache,id=modlight-cargo-registry,target=/usr/local/cargo/registry \
    cargo install cargo-c --version 0.10.25 --locked

# ── libvips: sharp-libvips 1.2.4's build, verbatim ────────────────────────────
FROM toolchain AS libvips
ARG TARGETARCH
COPY docker/sharp-libvips/ /packaging/
RUN mkdir -p /packaging/build \
 && cp /packaging/posix.sh /packaging/build/posix.sh \
 && cp /packaging/meson.ini /packaging/Toolchain.cmake /root/
# PLATFORM and FLAGS as platforms/linux-arm64v8 and platforms/linux-x64 set them.
# RUSTFLAGS there also carry two nightly-only size flags (-Zlocation-detail,
# -Zfmt-debug); this toolchain is stable, so they are left out (librsvg's panic
# strings stay in the binary; nothing it renders changes).
RUN set -e; \
    case "$TARGETARCH" in \
      arm64) export PLATFORM=linux-arm64v8 FLAGS="-march=armv8-a" ;; \
      amd64) export PLATFORM=linux-x64 FLAGS="-march=nehalem" ;; \
      *) echo "unsupported architecture: $TARGETARCH"; exit 1 ;; \
    esac; \
    export PKG_CONFIG="pkg-config --static" MESON="--cross-file=/root/meson.ini"; \
    bash /packaging/build/posix.sh > /packaging/build.log 2>&1 || { tail -n 80 /packaging/build.log; exit 1; }; \
    tail -n 5 /packaging/build.log; \
    cat /target/versions.json; \
    ln -s libvips-cpp.so.8.17.3 /target/lib/libvips-cpp.so; \
    mkdir -p /target/pkgconfig; \
    printf 'Name: vips\nDescription: libvips-cpp.so as sharp-libvips 1.2.4 ships it (libvips, GLib and GObject inside)\nVersion: 8.17.3\nLibs: -L/target/lib -lvips-cpp\n' > /target/pkgconfig/vips.pc

# ── server: the release binary ────────────────────────────────────────────────
FROM toolchain AS server
COPY --from=libvips /target/lib/ /target/lib/
COPY --from=libvips /target/pkgconfig/ /target/pkgconfig/
# build.rs links what `pkg-config --libs vips` names: here libvips-cpp.so.
ENV PKG_CONFIG_PATH=/target/pkgconfig CARGO_PROFILE_RELEASE_STRIP=symbols
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs rustfmt.toml ./
COPY src/ ./src/
COPY examples/ ./examples/
RUN --mount=type=cache,id=modlight-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=modlight-server-target,target=/src/target \
    cargo build --release --locked --bin modlight-server \
 && cp target/release/modlight-server /modlight-server

# ── runtime ───────────────────────────────────────────────────────────────────
FROM ${DEBIAN_IMAGE} AS runtime

# ffmpeg + ffprobe from Debian 12 (5.1, the reference image's major: the
# argument vectors upstream builds were written for it, SKELETON.md 9.33).
# --no-install-recommends as upstream: recommends would drag in X11 for ffplay.
# ffmpeg's dependencies already bring fontconfig's configuration and the DejaVu
# fonts (exactly what the reference image ends up with); they are named here so
# card rendering never depends on that accident: Pango falls back to them for
# glyphs Geist lacks. ca-certificates: rustls reads the system store (S3 over
# HTTPS, SMTP with TLS). libstdc++6: libvips-cpp.so is C++.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ffmpeg ca-certificates fontconfig-config fonts-dejavu-core libstdc++6 \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/*

COPY --from=libvips /target/lib/libvips-cpp.so.8.17.3 /usr/local/lib/
COPY --from=libvips /target/versions.json /usr/local/share/modlight/sharp-libvips-versions.json
RUN ldconfig

# uid/gid 1000 like the upstream image's `node` user, so a bind-mounted data
# directory owned by the first user of a Linux host just works.
RUN groupadd --gid 1000 modlight \
 && useradd --uid 1000 --gid 1000 --no-create-home --home-dir /app --shell /usr/sbin/nologin modlight

WORKDIR /app
COPY migrations/ /app/migrations/
COPY messages/ /app/messages/
COPY public/ /app/public/
COPY --from=web /src/web/dist/ /app/web/dist/
COPY --from=server /modlight-server /usr/local/bin/modlight-server
COPY docker/entrypoint.sh /usr/local/bin/modlight-entrypoint
RUN chmod 0755 /usr/local/bin/modlight-entrypoint \
 && mkdir -p /app/data/storage /app/data/tus /app/data/tmp \
 && chown -R 1000:1000 /app/data

# BIND_ADDR: inside the container only; compose publishes the port on the
# host's 127.0.0.1. PANGOCAIRO_BACKEND: the binary sets it too (SKELETON 9.34).
ENV NODE_ENV=production \
    MODLIGHT_ROOT=/app \
    BIND_ADDR=0.0.0.0 \
    PORT=3000 \
    STORAGE_LOCAL_PATH=/app/data/storage \
    TUS_TMP_PATH=/app/data/tus \
    UPLOAD_TMP_PATH=/app/data/tmp \
    PANGOCAIRO_BACKEND=fc

USER 1000:1000
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/modlight-entrypoint"]
CMD ["serve"]
