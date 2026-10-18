# syntax=docker/dockerfile:1

# ---- Web UI (built once, on the build machine's architecture) ----
FROM --platform=$BUILDPLATFORM node:22-alpine AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# ---- Rust binary (cross-compiled to a static musl binary for TARGETPLATFORM) ----
FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.6.1 AS xx
FROM --platform=$BUILDPLATFORM rust:1-alpine AS build
COPY --from=xx / /
RUN apk add --no-cache clang lld
ARG TARGETPLATFORM
RUN xx-apk add --no-cache musl-dev gcc
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY data/ data/
COPY --from=web /src/web/dist web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=target-$TARGETPLATFORM \
    xx-cargo build --release --locked \
    && cp target/$(xx-cargo --print-target-triple)/release/pixelgrove /pixelgrove \
    && xx-verify --static /pixelgrove

# ---- ffmpeg for video thumbnails: a small static build (cross-compiled like the binary)
# with just what's needed to read phone and camera video and write one JPEG frame. ----
FROM --platform=$BUILDPLATFORM alpine:3.21 AS ffmpeg
COPY --from=xx / /
RUN apk add --no-cache clang lld llvm musl-dev make nasm curl xz
ARG TARGETPLATFORM
RUN xx-apk add --no-cache musl-dev gcc
ARG FFMPEG_VERSION=8.1.2
ARG FFMPEG_SHA256=464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c
WORKDIR /src
RUN curl -fsSL -o ffmpeg.tar.xz https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz \
    && echo "${FFMPEG_SHA256}  ffmpeg.tar.xz" | sha256sum -c - \
    && tar xf ffmpeg.tar.xz --strip-components=1 && rm ffmpeg.tar.xz
RUN export PATH="$(echo /usr/lib/llvm*/bin):$PATH" \
    && ./configure \
      --enable-cross-compile --target-os=linux --arch=$(xx-info march) \
      --cc=xx-clang --cxx=xx-clang++ --ld=xx-clang --host-cc=clang \
      --ar=llvm-ar --nm=llvm-nm --ranlib=llvm-ranlib --strip=llvm-strip \
      --extra-ldflags=-static --enable-static --disable-shared \
      --disable-everything --disable-autodetect --disable-doc --disable-debug --disable-network \
      --disable-ffplay --disable-ffprobe --disable-avdevice --disable-swresample \
      --enable-protocol=file,pipe \
      --enable-demuxer=mov,matroska \
      --enable-decoder=h264,hevc,vp8,vp9,mpeg4,mjpeg,prores \
      --enable-parser=h264,hevc,vp8,vp9,mpeg4video,mjpeg \
      --enable-encoder=mjpeg --enable-muxer=mjpeg,image2pipe \
      --enable-filter=scale,format,transpose,hflip,vflip,null,copy \
    && make -j"$(nproc)" ffmpeg \
    && llvm-strip ffmpeg \
    && xx-verify --static ffmpeg \
    && cp ffmpeg /ffmpeg

# ---- Runtime: the binary and ffmpeg ----
FROM scratch
COPY --from=build /pixelgrove /pixelgrove
COPY --from=ffmpeg /ffmpeg /usr/local/bin/ffmpeg
ENV PIXELGROVE_CONFIG_DIR=/config \
    PIXELGROVE_FFMPEG=/usr/local/bin/ffmpeg \
    PIXELGROVE_MEDIA_ROOTS=/media \
    PIXELGROVE_BIND=0.0.0.0:8080 \
    MIMALLOC_ARENA_EAGER_COMMIT=0 \
    MIMALLOC_PURGE_DELAY=0
VOLUME ["/config"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["/pixelgrove", "healthcheck"]
ENTRYPOINT ["/pixelgrove"]
CMD ["serve"]
