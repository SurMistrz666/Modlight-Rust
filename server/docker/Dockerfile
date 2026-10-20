# syntax=docker/dockerfile:1.7
#
# Modlight server (Rust port), production image.
#
#   docker build -f server/docker/Dockerfile server        # context = server/
#
# Stages:
#   web        the SPA (server/web, Vite) -> /src/web/dist
#   libvips    libvips 8.17.3 built by sharp-libvips 1.2.4's own build script
#              (docker/sharp-libvips/posix.sh, unmodified) in the build image
#              sharp-libvips uses (Amazon Linux 2, gcc 10): the same library,
#              the same dependency versions, the same feature set and the
#              same compiler as the libvips-cpp.so the reference's sharp
#              0.34.5 loads, so even encoders whose output follows the
#              compiler (libaom's AVIF) write the same bytes
#   server     the release binary (Debian 12, Rust), linked against that
#              libvips-cpp.so
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

# ── libvips: sharp-libvips 1.2.4's build, verbatim ────────────────────────────
# The build image is platforms/linux-arm64v8 (or linux-x64)/Dockerfile of
# sharp-libvips v1.2.4: Amazon Linux 2 (glibc 2.26) with gcc 10. Differences:
# a pinned AL2 tag, a pinned stable Rust (sharp-libvips: nightly, for two
# size-only -Z flags that are left out: librsvg keeps its panic strings,
# nothing it renders changes) and pinned meson/ninja/cargo-c versions.
FROM amazonlinux:2.0.20260930.0 AS libvips
ARG TARGETARCH
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN set -e; \
    yum update -y; \
    amazon-linux-extras install epel -y; \
    extra=""; [ "$TARGETARCH" = amd64 ] && extra="nasm"; \
    yum install -y --setopt=tsflags=nodocs --exclude=gcc \
      autoconf automake bzip2 cmake3 curl gcc10 gcc10-binutils gcc10-c++ gettext git gperf jq make \
      patch pkgconfig python3 tar xz $extra; \
    yum clean all; \
    ln -s /usr/bin/gcc10-gcc /usr/bin/cc; \
    ln -s /usr/bin/gcc10-g++ /usr/bin/c++; \
    [ "$TARGETARCH" = arm64 ] && ln -s /usr/bin/gcc10-as /usr/bin/as; \
    ln -s /usr/bin/gcc10-ar /usr/bin/ar; \
    ln -s /usr/bin/gcc10-nm /usr/bin/nm; \
    ln -s /usr/bin/gcc10-ld.bfd /usr/bin/ld; \
    ln -s /usr/bin/gcc10-strip /usr/bin/strip; \
    ln -s /usr/bin/gcc10-ranlib /usr/bin/ranlib; \
    ln -s /usr/bin/gcc10-readelf /usr/bin/readelf; \
    ln -sf /usr/bin/gcc10-ld.bfd /usr/bin/gcc10-ld; \
    rm -f /usr/bin/gcc10-ld.gold; \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
      --default-toolchain 1.99.0; \
    ln -s /usr/bin/cmake3 /usr/bin/cmake; \
    pip3 install --no-cache-dir meson==1.9.1 ninja==1.11.1.4 tomli==2.0.1
# cargo-c with its own OpenSSL 3 (`vendored-openssl`): the cargo it embeds
# needs OpenSSL >= 1.1 and its vendored curl >= 3.0, and AL2 ships 1.0.2 (the
# sharp-libvips image of 2025 built an older cargo-c against that). OpenSSL's
# build needs Perl; Perl is removed again with what it brought along
# (systemtap's dtrace, which GLib would otherwise pick up for its probes), so
# posix.sh runs in the same image sharp-libvips used.
RUN --mount=type=cache,id=modlight-al2-cargo-registry,target=/usr/local/cargo/registry \
    set -e; \
    yum install -y --setopt=tsflags=nodocs perl-core perl-IPC-Cmd; \
    cargo install cargo-c --version 0.10.25 --locked --features vendored-openssl; \
    yum history undo -y last; \
    yum clean all; \
    if command -v dtrace; then echo "dtrace still installed"; exit 1; fi
COPY docker/sharp-libvips/ /packaging/
RUN mkdir -p /packaging/build \
 && cp /packaging/posix.sh /packaging/build/posix.sh \
 && cp /packaging/meson.ini /packaging/Toolchain.cmake /root/
# PLATFORM and FLAGS as platforms/linux-arm64v8 and platforms/linux-x64 set them.
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
FROM rust:1.99.0-slim-bookworm AS server
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config \
 && rm -rf /var/lib/apt/lists/*
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
