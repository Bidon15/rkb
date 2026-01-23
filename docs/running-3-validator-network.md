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
- `keys/validator-0.key`, `validator-1.key`, `validator-2.key` - Ed25519 validator keys
- `keys/jwt-0.hex`, `jwt-1.hex`, `jwt-2.hex` - JWT secrets for reth
- `keys/celestia.key` - Celestia signing key (needs funding!)

### 2. Fund Celestia Account (Required)

For blob submission to work, you need to fund the Celestia account:

1. Get the address from your celestia.key (secp256k1)
2. Visit the Mocha faucet: https://faucet.celestia-mocha.com/
3. Request testnet TIA tokens

### 3. Start the Network

```bash
cd docker
docker compose up -d
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

## Configuration

Each validator has its own config in `docker/config/validator-N/config.toml`.

### Key Configuration Options

```toml
[consensus]
# All validators must agree on the same validator_seeds
validator_seeds = [0, 1, 2]

# Each validator uses its own seed
validator_seed = 0  # 0, 1, or 2

# P2P peers (other validators)
peers = [
    "1@validator-1:26656",
    "2@validator-2:26656",
]

# Timeouts
leader_timeout_ms = 2000      # Time to wait for leader's proposal
notarization_timeout_ms = 3000 # Time to collect 2/3 votes
nullify_retry_ms = 10000       # Retry interval for null blocks
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
   - 2/3+ votes = notarized

3. **Finalization** (3-hop):
   - Notarized block triggers finalization votes
   - 2/3+ finalization votes = finalized
   - Leader submits finalized block to Celestia

### Fault Tolerance

With 3 validators (n=3, f=1):
- Tolerates 1 Byzantine validator
- Requires 2/3+ (≥2) honest validators for progress
- If leader is faulty, view change after timeout

## Troubleshooting

### Validators Not Connecting

Check P2P connectivity:
```bash
docker compose logs validator-0 | grep -i "peer\|connect"
```

Verify all validators are on the same Docker network:
```bash
docker network inspect docker_sequencer-net
```

### Celestia Submission Failing

1. Check if Celestia key is funded
2. Verify RPC/gRPC endpoints are reachable
3. Check logs for specific errors:
   ```bash
   docker compose logs validator-0 | grep -i "celestia\|submit"
   ```

### reth Not Starting

Check JWT secret matches:
```bash
cat docker/keys/jwt-0.hex
docker compose logs reth-0 | grep -i "jwt\|auth"
```

### View Current Block Height

```bash
# Check validator logs for finalized blocks
docker compose logs validator-0 | grep "Block finalized"
```

## Development

### Building Locally

```bash
cargo build --release
./target/release/sequencer simplex -c config.toml
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
