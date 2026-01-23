# Running a 3-Validator PoA Network

This guide walks you through running a 3-validator Simplex BFT network locally using Docker Compose.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    3-Validator PoA Network                          │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐            │
│  │ Validator 0  │◄─►│ Validator 1  │◄─►│ Validator 2  │            │
│  │  (Leader 0)  │   │  (Leader 1)  │   │  (Leader 2)  │            │
│  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘            │
│         │                  │                  │                      │
│    ┌────▼────┐        ┌────▼────┐        ┌────▼────┐               │
│    │ reth-0  │        │ reth-1  │        │ reth-2  │               │
│    │ (EVM)   │        │ (EVM)   │        │ (EVM)   │               │
│    └─────────┘        └─────────┘        └─────────┘               │
│                                                                      │
│                    P2P Mesh (commonware-p2p)                        │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │    Celestia     │
                    │  (Mocha Testnet)│
                    └─────────────────┘
```

## Prerequisites

- Docker and Docker Compose
- ~8GB RAM available
- Internet access (for Celestia Mocha testnet)

## Quick Start

### 1. Generate Keys

```bash
cd docker
./scripts/generate-keys.sh
```

This creates:

| Key Type | Files | Purpose |
|----------|-------|---------|
| Ed25519 Validator | `validator-{0,1,2}.key` | Consensus message signing |
| secp256k1 Celestia | `celestia-{0,1,2}.key` | Blob submission (PayForBlobs) |
| JWT Secrets | `jwt-{0,1,2}.hex` | reth Engine API auth |

### 2. Fund ALL THREE Celestia Accounts (Required)

Each validator needs its own funded Celestia account because any validator can become the leader and submit blobs.

**Get addresses:**
```bash
cargo build --release -p sequencer
../target/release/sequencer celestia-address --key keys/celestia-0.key
../target/release/sequencer celestia-address --key keys/celestia-1.key
../target/release/sequencer celestia-address --key keys/celestia-2.key
```

**Fund all 3 at:** https://faucet.celestia-mocha.com/

**Why 3 keys?** The leader rotates each block. When Validator 1 is leader, it uses `celestia-1.key` to submit. Without funding, that validator's blob submissions will fail.

### 3. Start the Network

```bash
docker compose up -d --build
```

This starts:
- 3 sequencer validators
- 3 reth instances

### 4. Check Logs

```bash
# All validators
docker compose logs -f

# Specific validator
docker compose logs -f validator-0
```

### 5. Stop the Network

```bash
docker compose down

# To also remove volumes (data):
docker compose down -v
```

## Testing & Verification

### Block Production Timeline

```
Time 0ms:     Leader proposes block
Time ~100ms:  2/3 validators vote → block notarized (soft finality)
Time ~200ms:  Block executed on reth, forkchoice updated
Time 5000ms:  Batch of ~25-50 blocks submitted to Celestia
Time ~6s:     Celestia includes blob → firm finality
```

### Verify Consensus is Working

```bash
# Watch block execution (new blocks every ~100-200ms)
docker compose logs -f | grep -E "Executing block|executed successfully"

# Check leader rotation
docker compose logs -f | grep -i "leader"
```

### Verify Celestia Batching

```bash
# Watch batch submissions (every 5 seconds)
docker compose logs -f | grep -i "batch"

# Expected output:
# "Submitting block batch to Celestia" block_count=25
# "Block submitted to Celestia"
# "Block finalized on Celestia"
```

### Verify reth Execution

```bash
# Check Engine API calls
docker compose logs -f validator-0 | grep -i "payload\|forkchoice"

# Expected output:
# "Executing block via Engine API"
# "Block executed"
# "Forkchoice updated with Celestia finality"
```

### Check Network Health

```bash
# All services running?
docker compose ps

# Peer connections (should show 2 peers per validator)
docker compose logs validator-0 | grep -i "peer"

# Block count
docker compose logs validator-0 | grep "Block executed" | wc -l
```

## Configuration

Each validator has its own config in `docker/config/validator-N/config.toml`.

### Consensus Timing

```toml
[consensus]
leader_timeout_ms = 2000       # Wait for leader's proposal
notarization_timeout_ms = 3000 # Wait for 2/3 votes
nullify_retry_ms = 10000       # Retry interval for null blocks
```

### Celestia Batching

```toml
[celestia]
# Time-based: submit every N milliseconds
batch_interval_ms = 5000

# Size-based: submit early if batch exceeds N bytes
max_batch_size_bytes = 1500000  # 1.5MB

# Gas price for submissions
gas_price = 0.002
```

**Batching behavior:**
- Blocks accumulate until either trigger fires
- `batch_interval_ms` - time limit (default 5 seconds)
- `max_batch_size_bytes` - size limit (default 1.5MB)
- Whichever comes first triggers submission

### P2P Configuration

```toml
[consensus]
# All validators must have the same validator_seeds
validator_seeds = [0, 1, 2]

# Each validator uses its own seed
validator_seed = 0  # 0, 1, or 2

# Bootstrap peers (other validators)
peers = [
    "1@validator-1:26656",
    "2@validator-2:26656",
]
```

## How It Works

### Consensus Flow

1. **Leader Election**: Round-robin based on block height
   - Height 0: Validator 0 is leader
   - Height 1: Validator 1 is leader
   - Height 2: Validator 2 is leader
   - Height 3: Validator 0 is leader (wraps around)

2. **Block Production** (2-hop):
   - Leader proposes a block
   - All validators vote
   - 2/3+ votes = notarized (soft finality)

3. **Execution**:
   - Each validator executes the block on reth
   - Forkchoice updated (head/safe)

4. **Celestia Submission**:
   - Leader accumulates blocks in batch
   - Every 5 seconds (or when size limit hit), submits batch
   - All blocks in batch share single PayForBlobs transaction

5. **Firm Finality**:
   - Celestia includes the blob
   - Finality tracker detects inclusion
   - Forkchoice updated (finalized)

### Finality Levels

| Level | Trigger | Latency | Guarantees |
|-------|---------|---------|------------|
| Soft | 2/3 notarization | ~100-200ms | BFT consensus agreement |
| Firm | Celestia inclusion | ~6s | Data availability proven |

### Fault Tolerance

With 3 validators (n=3, f=1):
- Tolerates 1 Byzantine validator
- Requires 2/3+ (≥2) honest validators for progress
- If leader is faulty, view change after timeout

## Troubleshooting

### Validators Not Connecting

```bash
# Check P2P connectivity
docker compose logs validator-0 | grep -i "peer\|connect"

# Verify network
docker network inspect docker_sequencer-net
```

### Celestia Submission Failing

1. Check if Celestia key is funded
2. Verify RPC/gRPC endpoints are reachable
3. Check logs:
   ```bash
   docker compose logs validator-0 | grep -i "celestia\|submit\|failed"
   ```

### reth Not Starting

```bash
# Check JWT secret matches
cat docker/keys/jwt-0.hex
docker compose logs reth-0 | grep -i "jwt\|auth"

# Check genesis loaded
docker compose logs reth-0 | grep -i "genesis\|chain"
```

### No Blocks Being Produced

- Need 2/3 validators (2 of 3) online
- Check all running: `docker compose ps`
- Check for errors: `docker compose logs | grep -i error`

## Development

### Building Locally

```bash
cargo build --release
./target/release/sequencer run -c config.toml
```

### Running Tests

```bash
cargo test --workspace
```

### Modifying Validator Count

To change the number of validators:

1. Update `validator_seeds` in all configs
2. Add/remove services in `docker-compose.yml`
3. Update `peers` in each config to reference all other validators
4. Generate additional keys with `generate-keys.sh`

## Network Ports

| Service | Port | Description |
|---------|------|-------------|
| validator-0 | 26656 | P2P |
| validator-1 | 26657 | P2P |
| validator-2 | 26658 | P2P |
| reth-0 | 8545 (internal) | JSON-RPC |
| reth-0 | 8551 (internal) | Engine API |

## Security Notes

- The generated keys use deterministic seeds for testing only
- In production, generate proper random keys
- The Celestia key must be securely managed in production
- Consider using hardware security modules (HSM) for validator keys
