FROM rust AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=target \
    cargo build --release \
    && cp target/release/prosze-hackowac prosze-hackowac

FROM debian:stable-slim
COPY static static
COPY --from=builder /app/prosze-hackowac /app/
ENTRYPOINT ["/app/prosze-hackowac"]
