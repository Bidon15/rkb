# RKB Docker Setup

3-validator PoA network using Simplex BFT consensus with Celestia DA.

## Key Insight: Transactions Go to reth

```
Users ──► reth JSON-RPC (port 8545) ──► reth mempool ──► sequencer builds blocks
```

**You don't send transactions to the sequencer.** Send them directly to any reth instance using standard Ethereum tooling (`eth_sendRawTransaction`). The sequencer has no mempool—it orchestrates reth, which handles all transaction management.

## Quick Start

```bash
cd docker

# 1. Generate keys
./scripts/generate-keys.sh

# 2. Get Celestia addresses (build first)
cargo build --release -p sequencer
../target/release/sequencer celestia-address --key keys/celestia-0.key
../target/release/sequencer celestia-address --key keys/celestia-1.key
../target/release/sequencer celestia-address --key keys/celestia-2.key

# 3. Fund ALL THREE addresses at https://faucet.celestia-mocha.com/

# 4. Build and run
docker compose up -d --build

# 5. Watch logs
docker compose logs -f
```

## Sending Transactions

Send transactions to **any reth instance** using standard Ethereum tools:

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

**Why reth, not sequencer?**
- reth has a production-grade mempool (we wrote 0 lines)
- reth handles nonce ordering, gas price priority (we wrote 0 lines)
- reth validates transactions before inclusion (we wrote 0 lines)
- Standard Ethereum tooling just works

## Testing the Network

### Verify Validators Connect

```bash
# Check P2P connections (should see 2 peers)
docker compose logs validator-0 | grep -i "peer"

# Check consensus is running
docker compose logs validator-0 | grep -i "leader\|proposal\|notari"
```

### Watch Block Production

```bash
# See block execution
docker compose logs -f | grep -E "height|executed|finalized"

# Check specific validator
docker compose logs -f validator-0 | grep "Block"
```

### Check Celestia Submissions

```bash
# Watch batch submissions (every 5 seconds by default)
docker compose logs -f | grep -i "celestia\|batch\|submit"

# Successful submission looks like:
# "Submitting block batch to Celestia"
# "Block submitted to Celestia"
# "Block finalized on Celestia"
```

### Check reth (EVM) Status

```bash
# View reth logs
docker compose logs reth-0

# Check Engine API is working
docker compose logs validator-0 | grep -i "engine\|payload\|forkchoice"
```

### View Current State

```bash
# Count finalized blocks
docker compose logs validator-0 | grep "Block finalized" | wc -l

# Check latest block height
docker compose logs validator-0 | grep "height" | tail -5
```

## Configuration

### Batch Settings

In `config/validator-*/config.toml`:

```toml
[celestia]
# Submit every 5 seconds
batch_interval_ms = 5000

# Or submit early if batch exceeds 1.5MB
max_batch_size_bytes = 1500000
```

### Consensus Timing

```toml
[consensus]
leader_timeout_ms = 2000       # Wait for leader proposal
notarization_timeout_ms = 3000 # Wait for 2/3 votes
nullify_retry_ms = 10000       # Retry on failed rounds
```

## Commands

```bash
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
- Check: `../target/release/sequencer celestia-address --key keys/celestia-0.key`
- Get tokens: https://faucet.celestia-mocha.com/

### "Failed to execute block"

- Check reth is running: `docker compose logs reth-0`
- Verify JWT auth: `docker compose logs validator-0 | grep -i jwt`

### Validators Not Connecting

- Check network: `docker network inspect docker_sequencer-net`
- Verify peers in config match container names

### No Blocks Being Produced

- Need 2/3 validators (2 of 3) for consensus
- Check all 3 are running: `docker compose ps`

## Structure

```
docker/
├── config/
│   ├── validator-0/config.toml
│   ├── validator-1/config.toml
│   └── validator-2/config.toml
├── keys/                  # Generated, gitignored
├── scripts/
│   └── generate-keys.sh
├── genesis.json           # reth genesis (EVM chain config)
├── docker-compose.yml
├── Dockerfile
└── README.md
```

## Ports

| Service | Host Port | Description |
|---------|-----------|-------------|
| reth-0 | 8545 | **JSON-RPC (send transactions here)** |
| reth-1 | 8546 | JSON-RPC (alternate) |
| reth-2 | 8547 | JSON-RPC (alternate) |
| validator-0 | 26656 | P2P consensus |
| validator-1 | 26657 | P2P consensus |
| validator-2 | 26658 | P2P consensus |

Engine API ports (8551) are internal only—validators use them to communicate with their reth instances.

---

See [docs/running-3-validator-network.md](../docs/running-3-validator-network.md) for architecture details.
