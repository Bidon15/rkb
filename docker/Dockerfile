# Build stage
FROM rust:1.85-bookworm as builder

WORKDIR /app
COPY . .

RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sequencer /usr/local/bin/

ENTRYPOINT ["sequencer"]
CMD ["run", "--config", "/config/config.toml"]
