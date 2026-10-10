# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.96.1
FROM node:22-bookworm-slim AS frontend
WORKDIR /build/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:${RUST_VERSION}-bookworm AS backend
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY src/ ./src/
COPY --from=frontend /build/frontend/dist/ ./frontend/dist/
RUN PICSOC_FRONTEND_PREBUILT=1 cargo build --locked --release

# Export the Debian 12-built executable for the Linux release archive.
FROM scratch AS binary
COPY --from=backend /build/target/release/picsoc /picsoc

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends fonts-wqy-zenhei fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 picsoc \
    && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /data picsoc \
    && mkdir -p /data /library \
    && chown picsoc:picsoc /data
COPY --from=backend /build/target/release/picsoc /usr/local/bin/picsoc
COPY LICENSE README.md README.en.md /usr/share/doc/picsoc/
ENV PICSOC_BIND=0.0.0.0:3210 \
    PICSOC_DATA_DIR=/data \
    PICSOC_WORKERS=1 \
    PICSOC_SCAN_INTERVAL=300
USER 10001:10001
WORKDIR /data
EXPOSE 3210
ENTRYPOINT ["/usr/local/bin/picsoc"]
CMD ["--no-open"]
