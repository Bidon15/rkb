# RKB Docker Setup

3-validator PoA network using Simplex BFT consensus with Celestia DA.

## Choose Your Execution Client

| Setup | Block Time | Client | Use Case |
|-------|------------|--------|----------|
| **[reth/](reth/)** | ~1 second | Vanilla Reth | Stability, standard Ethereum compatibility |
| **[ethrex/](ethrex/)** | ~33ms | Forked ethrex | Speed, sub-second confirmations |

See [block-timing-explained.md](../docs/block-timing-explained.md) for why this matters.

## Key Insight: Transactions Go to the Execution Client

```
Users --> Execution Client (port 8545) --> mempool --> sequencer builds blocks
```

**You don't send transactions to the sequencer.** Send them directly to any execution client instance using standard Ethereum tooling (`eth_sendRawTransaction`). The sequencer has no mempool—it orchestrates execution, which handles all transaction management.

## Quick Start

### Option 1: Vanilla Reth (1s blocks)

```bash
cd docker

# 1. Generate keys (shared by both setups)
./scripts/generate-keys.sh

# 2. Fund Celestia accounts (see below)

# 3. Start network
cd reth
docker compose up -d

# 4. Watch logs
docker compose logs -f
```

### Option 2: Forked ethrex (subsecond blocks)

```bash
cd docker

# 1. Generate keys (shared by both setups)
./scripts/generate-keys.sh

# 2. Build ethrex (from your ethrex fork with timestamp changes)
# cd /path/to/ethrex && docker build -t ethrex:latest .

# 3. Fund Celestia accounts (see below)

# 4. Start network
cd ethrex
docker compose up -d

# 5. (Optional) Connect ethrex peers for tx gossip
../scripts/connect-ethrex-peers.sh

# 6. Watch logs
docker compose logs -f
```

### Fund ALL THREE Celestia Accounts

Each validator needs its own funded Celestia account because any validator can become leader:

```bash
cargo build --release -p sequencer
../target/release/sequencer celestia-address --key keys/celestia-0.key
../target/release/sequencer celestia-address --key keys/celestia-1.key
../target/release/sequencer celestia-address --key keys/celestia-2.key
```

Fund all 3 at: https://faucet.celestia-mocha.com/

## Directory Structure

```
docker/
├── reth/                    # Vanilla Reth setup (1s blocks)
│   ├── config/validator-*/  # Validator configs (block_timing = "vanilla")
│   ├── docker-compose.yml   # Uses reth:v1.3.12
│   └── genesis.json         # Standard format
│
├── ethrex/                  # Forked ethrex setup (subsecond blocks)
│   ├── config/validator-*/  # Validator configs (block_timing = "subsecond")
│   ├── docker-compose.yml   # Uses ethrex:latest
│   └── genesis.json         # Has depositContractAddress
│
├── keys/                    # Shared keys (generated once)
│   ├── validator-*.key      # Ed25519 consensus keys
│   ├── celestia-*.key       # secp256k1 Celestia keys
│   └── jwt-*.hex            # JWT secrets for Engine API
│
└── scripts/                 # Shared scripts
    ├── generate-keys.sh     # Key generation
    ├── connect-reth-peers.sh
    └── connect-ethrex-peers.sh
```

## Sending Transactions

Send transactions to **any execution client instance** using standard Ethereum tools:

```bash
# Using cast (foundry)
cast send --rpc-url http://localhost:8545 \
  --private-key <YOUR_KEY> \
  <TO_ADDRESS> \
  --value 1ether

# Deploy a contract
forge create --rpc-url http://localhost:8545 \
  --private-key <YOUR_KEY> \
  src/MyContract.sol:MyContract

# Check balance
cast balance --rpc-url http://localhost:8545 <ADDRESS>

# Get block number
cast block-number --rpc-url http://localhost:8545
```

## Ports

| Service | Host Port | Description |
|---------|-----------|-------------|
| reth-0 / ethrex-0 | 8545 | **JSON-RPC (send transactions here)** |
| reth-1 / ethrex-1 | 8546 | JSON-RPC (alternate) |
| reth-2 / ethrex-2 | 8547 | JSON-RPC (alternate) |
| validator-0 | 26656 | P2P consensus |
| validator-1 | 26657 | P2P consensus |
| validator-2 | 26658 | P2P consensus |

Engine API ports (8551) are internal only.

## Commands

```bash
# From reth/ or ethrex/ directory:

# Logs
docker compose logs -f
docker compose logs -f validator-0

# Stop
docker compose down

# Reset (removes all data)
docker compose down -v

# Rebuild after code changes
docker compose up -d --build
```

## Troubleshooting

### "Celestia submission failed"

- Verify all 3 Celestia accounts are funded
- Check: `../target/release/sequencer celestia-address --key ../keys/celestia-0.key`
- Get tokens: https://faucet.celestia-mocha.com/

### "Failed to execute block"

- Check execution client is running: `docker compose logs reth-0` or `docker compose logs ethrex-0`
- Verify JWT auth: `docker compose logs validator-0 | grep -i jwt`

### ethrex "SYNCING" status

- Ensure `--syncmode=full` is set (already configured in ethrex/docker-compose.yml)
- The default "snap" mode returns SYNCING for private networks

### Validators Not Connecting

- Check network: `docker network inspect <project>_sequencer-net`
- Verify peers in config match container names

### No Blocks Being Produced

- Need 2/3 validators (2 of 3) for consensus
- Check all 3 are running: `docker compose ps`

---

See [docs/running-3-validator-network.md](../docs/running-3-validator-network.md) for architecture details.
