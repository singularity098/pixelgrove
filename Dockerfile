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
COPY --from=web /src/web/dist web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=target-$TARGETPLATFORM \
    xx-cargo build --release --locked \
    && cp target/$(xx-cargo --print-target-triple)/release/pixelgrove /pixelgrove \
    && xx-verify --static /pixelgrove

# ---- Runtime: just the binary ----
FROM scratch
COPY --from=build /pixelgrove /pixelgrove
ENV PIXELGROVE_CONFIG_DIR=/config \
    PIXELGROVE_MEDIA_ROOTS=/media \
    PIXELGROVE_BIND=0.0.0.0:8080 \
    MIMALLOC_ARENA_EAGER_COMMIT=0 \
    MIMALLOC_PURGE_DELAY=0
VOLUME ["/config"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["/pixelgrove", "healthcheck"]
ENTRYPOINT ["/pixelgrove"]
CMD ["serve"]
