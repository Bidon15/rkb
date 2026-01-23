# Task 06: Infrastructure

## Owner
Agent Session 4 (parallel with contracts)

## Goal
Docker images, deployment configs, and documentation for customers.

## Deliverable
- Dockerfile for sequencer
- docker-compose.yml for full stack
- Deployment documentation

---

## Docker

### Sequencer Dockerfile

```dockerfile
# Dockerfile
FROM rust:1.75-slim as builder

WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sequencer /usr/local/bin/

ENTRYPOINT ["sequencer"]
CMD ["run", "--config", "/config/config.toml"]
```

### Docker Compose

```yaml
# docker-compose.yml
version: "3.8"

services:
  sequencer:
    image: your-org/sequencer:latest
    container_name: sequencer
    restart: unless-stopped
    ports:
      - "26656:26656"  # P2P
      - "8080:8080"    # RPC (optional)
    volumes:
      - ./config:/config:ro
      - sequencer-data:/data
    environment:
      - RUST_LOG=info
    depends_on:
      - reth
    networks:
      - chain

  reth:
    image: ghcr.io/paradigmxyz/reth:latest
    container_name: reth
    restart: unless-stopped
    command: >
      node
      --chain=/config/genesis.json
      --datadir=/data
      --http
      --http.addr=0.0.0.0
      --http.port=8545
      --http.api=eth,net,web3,debug,trace
      --authrpc.addr=0.0.0.0
      --authrpc.port=8551
      --authrpc.jwtsecret=/config/jwt.hex
    ports:
      - "8545:8545"    # JSON-RPC
      - "8551:8551"    # Engine API (internal only in prod)
    volumes:
      - ./config:/config:ro
      - reth-data:/data
    networks:
      - chain

volumes:
  sequencer-data:
  reth-data:

networks:
  chain:
    driver: bridge
```

---

## Configuration Templates

### config/config.toml

```toml
# Sequencer configuration
chain_id = 12345

[consensus]
validators = [
    "0x1111111111111111111111111111111111111111",
    "0x2222222222222222222222222222222222222222",
    "0x3333333333333333333333333333333333333333",
]
private_key_path = "/config/validator.key"
listen_addr = "0.0.0.0:26656"
peers = []  # Add other validator addresses

[celestia]
network = "mainnet"
namespace = "my_chain_namespace"

[execution]
reth_url = "http://reth:8551"
jwt_secret_path = "/config/jwt.hex"

[rpc]
listen_addr = "0.0.0.0:8080"
```

### config/genesis.json

```json
{
  "config": {
    "chainId": 12345,
    "homesteadBlock": 0,
    "eip150Block": 0,
    "eip155Block": 0,
    "eip158Block": 0,
    "byzantiumBlock": 0,
    "constantinopleBlock": 0,
    "petersburgBlock": 0,
    "istanbulBlock": 0,
    "berlinBlock": 0,
    "londonBlock": 0,
    "shanghaiTime": 0,
    "cancunTime": 0,
    "terminalTotalDifficulty": 0,
    "terminalTotalDifficultyPassed": true
  },
  "nonce": "0x0",
  "timestamp": "0x0",
  "extraData": "0x",
  "gasLimit": "0x1c9c380",
  "difficulty": "0x0",
  "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "coinbase": "0x0000000000000000000000000000000000000000",
  "alloc": {
    "0x1111111111111111111111111111111111111111": {
      "balance": "0x152d02c7e14af6800000"
    }
  }
}
```

---

## Tasks

### 1. Docker
- [ ] Create Dockerfile
- [ ] Create docker-compose.yml
- [ ] Test local build
- [ ] Multi-arch builds (amd64, arm64)

### 2. Configuration Templates
- [ ] config.toml template
- [ ] genesis.json template
- [ ] JWT secret generation script

### 3. Scripts
- [ ] `scripts/init.sh` - Initialize new deployment
- [ ] `scripts/keygen.sh` - Generate validator keys
- [ ] `scripts/genesis.sh` - Generate genesis file

### 4. Documentation
- [ ] `docs/quickstart.md` - 5-minute setup
- [ ] `docs/deployment.md` - Production deployment
- [ ] `docs/configuration.md` - All config options
- [ ] `docs/validator-guide.md` - For validators
- [ ] `docs/bridging.md` - Hyperlane setup

### 5. CI/CD
- [ ] GitHub Actions for build
- [ ] Docker image publishing
- [ ] Release automation

---

## Scripts

### scripts/init.sh

```bash
#!/bin/bash
set -e

echo "Initializing new chain deployment..."

# Create directories
mkdir -p config data

# Generate JWT secret
openssl rand -hex 32 > config/jwt.hex
echo "Generated JWT secret"

# Generate validator key
./sequencer keygen --output config/validator.key
echo "Generated validator key"

# Copy config template
cp templates/config.toml config/config.toml
echo "Copied config template - please edit config/config.toml"

echo ""
echo "Next steps:"
echo "1. Edit config/config.toml with your settings"
echo "2. Generate genesis: ./sequencer genesis --config config/genesis-config.toml --output config/genesis.json"
echo "3. Start: docker-compose up -d"
```

### scripts/keygen.sh

```bash
#!/bin/bash
set -e

OUTPUT=${1:-"validator.key"}

./sequencer keygen --output "$OUTPUT"
echo "Generated validator key: $OUTPUT"

# Extract public key
./sequencer pubkey --key "$OUTPUT"
```

---

## Documentation

### docs/quickstart.md

```markdown
# Quickstart

Deploy your PoA chain in 5 minutes.

## Prerequisites

- Docker and Docker Compose
- USDC on Ethereum (for bridging)

## Steps

### 1. Initialize

\`\`\`bash
git clone https://github.com/your-org/poa-chain
cd poa-chain
./scripts/init.sh
\`\`\`

### 2. Configure

Edit \`config/config.toml\`:
- Set your chain ID
- Add validator addresses
- Configure Celestia network

### 3. Generate Genesis

\`\`\`bash
./sequencer genesis --config config/genesis-config.toml --output config/genesis.json
\`\`\`

### 4. Start

\`\`\`bash
docker-compose up -d
\`\`\`

### 5. Verify

\`\`\`bash
# Check sequencer logs
docker-compose logs -f sequencer

# Check reth
curl http://localhost:8545 -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
\`\`\`

## Next Steps

- [Bridge USDC](./bridging.md) - Get native tokens
- [Add Validators](./validator-guide.md) - Expand your network
- [Production Deployment](./deployment.md) - Harden for production
```

---

## CI/CD

### .github/workflows/build.yml

```yaml
name: Build

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Build
        run: cargo build --release

      - name: Test
        run: cargo test

      - name: Build Docker
        run: docker build -t sequencer:latest .

  publish:
    needs: build
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Login to GHCR
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Build and Push
        uses: docker/build-push-action@v5
        with:
          push: true
          tags: ghcr.io/${{ github.repository }}:latest
```

---

## Output

When complete:

```
.
├── Dockerfile
├── docker-compose.yml
├── scripts/
│   ├── init.sh
│   ├── keygen.sh
│   └── genesis.sh
├── templates/
│   ├── config.toml
│   └── genesis-config.toml
├── docs/
│   ├── quickstart.md
│   ├── deployment.md
│   ├── configuration.md
│   ├── validator-guide.md
│   └── bridging.md
└── .github/
    └── workflows/
        └── build.yml
```
