# Build stage
FROM rust:1.85-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release --bin sequencer

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sequencer /usr/local/bin/

# Create directories for config and data
RUN mkdir -p /config /data /keys

ENTRYPOINT ["sequencer"]
CMD ["simplex", "--config", "/config/config.toml"]
