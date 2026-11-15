# syntax=docker/dockerfile:1

# Build a static musl binary, then ship it alone in an empty image.
FROM rust:1.99.1-alpine AS build
RUN apk add --no-cache musl-dev
# Topcoat downloads a glibc Tailwind CLI, which does not run on Alpine, so the
# musl build is fetched here, pinned by hash. x86_64 only: for arm64 use the
# linux-arm64-musl release and its hash.
ARG TAILWIND_VERSION=4.3.2
ARG TAILWIND_SHA256=ae828e9e989ecbddb2bef856af8b0308ba162583b4922b3a065b5e26f86b0691
RUN wget -q -O /usr/local/bin/tailwindcss "https://github.com/tailwindlabs/tailwindcss/releases/download/v${TAILWIND_VERSION}/tailwindcss-linux-x64-musl" \
    && echo "${TAILWIND_SHA256}  /usr/local/bin/tailwindcss" | sha256sum -c - \
    && chmod +x /usr/local/bin/tailwindcss
ENV TAILWIND_CLI=/usr/local/bin/tailwindcss
WORKDIR /app
# The context is the repository root; the other members are only read by Cargo
# to resolve the shared lock file (see Dockerfile.dockerignore).
#
# The dependencies come first, built from the manifests and empty sources, so
# this layer (the slow one) is rebuilt only when a manifest or the lock file
# changes, not for every edit of a page.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/bufonidae-core/Cargo.toml crates/bufonidae-core/Cargo.toml
COPY crates/bufonidae-rclone/Cargo.toml crates/bufonidae-rclone/Cargo.toml
COPY crates/bufonidae-cli/Cargo.toml crates/bufonidae-cli/Cargo.toml
COPY apps/desktop/Cargo.toml apps/desktop/Cargo.toml
COPY website/Cargo.toml website/Cargo.toml
COPY vendor vendor
# build.rs reads these product files for the site's facts, so they are real
# here, not stubs.
COPY crates/bufonidae-rclone/src/lib.rs crates/bufonidae-rclone/src/lib.rs
COPY crates/bufonidae-cli/src/main.rs crates/bufonidae-cli/src/main.rs
RUN mkdir -p crates/bufonidae-core/src apps/desktop/src website/src \
    && echo 'fn main() {}' >apps/desktop/src/main.rs \
    && echo 'fn main() {}' >website/src/main.rs \
    && echo 'fn main() {}' >website/build.rs \
    && : >crates/bufonidae-core/src/lib.rs && : >website/src/lib.rs \
    && cargo build --profile website --locked -p bufonidae-website

# The sources. Their modification times can be older than the stubs above, so
# they are touched, or Cargo would keep the stubs' build.
COPY website/build.rs website/styles.css website/
COPY website/src website/src
COPY website/assets website/assets
RUN find website/src website/build.rs -name '*.rs' -exec touch {} + \
    && cargo build --profile website --locked -p bufonidae-website \
    && cp target/website/bufonidae-website /server

FROM scratch
COPY --from=build /server /server
ENV HOST=0.0.0.0 PORT=3000
EXPOSE 3000
USER 10001:10001
HEALTHCHECK --interval=30s --timeout=3s --start-period=2s --retries=3 \
    CMD ["/server", "--health"]
ENTRYPOINT ["/server"]
