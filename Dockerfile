# Build stage
FROM rust:1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release --bin ssdm

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 ssdm \
    && mkdir -p /data && chown ssdm:ssdm /data
COPY --from=builder /app/target/release/ssdm /usr/local/bin/ssdm
USER ssdm
VOLUME ["/data"]
ENV RUST_LOG=info
ENTRYPOINT ["ssdm"]
CMD ["daemon"]
