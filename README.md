# Celestia-native PoA EVM Sequencer

A minimal sequencer stack for PoA EVM chains using Celestia for data availability.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Sequencer Node                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │  Consensus  │→→│  Execution  │→→│      Celestia       │ │
│  │    (PoA)    │  │ (Engine API)│  │  (Lumina Client)    │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
│         ↓               ↓                    ↓              │
│    Block Prod      reth Node           Blob Submit         │
└─────────────────────────────────────────────────────────────┘
```

## Features

- **PoA Consensus**: Simple block production with configurable validators
- **Vanilla reth**: Standard Ethereum execution via Engine API
- **Celestia DA**: Block data stored on Celestia, finality from L1
- **Two-tier Finality**: Instant soft + ~12s firm confirmation
- **Custom Native Token**: USDC, TIA, or any token as gas

## Quick Start

```bash
# Build
cargo build --release

# Generate config
./target/release/sequencer init --output config.toml

# Generate validator key
./target/release/sequencer keygen --output validator.key

# Run
./target/release/sequencer run --config config.toml
```

## Configuration

```toml
[chain]
chain_id = 1337
block_time_ms = 1000
gas_limit = 30000000

[consensus]
validators = ["0x..."]
private_key_path = "validator.key"

[celestia]
submit_endpoint = "http://localhost:26658"
query_endpoint = "http://localhost:26659"
namespace = "sequencer"

[execution]
reth_url = "http://localhost:8551"
jwt_secret_path = "jwt.hex"
```

## Project Structure

```
crates/
├── sequencer/     # Main binary
├── consensus/     # PoA block production
├── celestia/      # Lumina client + finality tracking
├── execution/     # Engine API client
└── types/         # Shared types
```

## Development

```bash
# Check
cargo check --workspace

# Test
cargo test --workspace

# Lint
cargo clippy --workspace

# Format
cargo fmt
```

## License

MIT OR Apache-2.0
