# OSS-Fuzz Dockerfile for ts-transformer.
#
# base-builder-rust ships rustup + nightly + cargo-fuzz.
# We add cmake (libsrt build) and pkg-config (build-script probe path).
# Submodule clone pulls vendor/srt (libsrt 1.5.5) and vendor/mbedtls (3.6.x).
FROM gcr.io/oss-fuzz-base/base-builder-rust

RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --depth 1 --recurse-submodules \
    https://github.com/aklofas/ts-transformer "$SRC/ts-transformer"

WORKDIR $SRC/ts-transformer

COPY build.sh $SRC/
