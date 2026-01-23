# RKB (Rauh-Konsens Begriff)

A raw coordination stack that enforces only minimal mechanical truth.

---

## Architecture

```
Ordering     →  PoA Consensus (commonware-consensus)
Availability →  Celestia DA (celestia-client)
Execution    →  reth via Engine API
```

Three components wired together. No reinvented wheels.

## What This Gives You

- Block ordering
- Data availability
- EVM execution
- Nothing else

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

For multi-validator setups, see [Running a 3-Validator Network](docs/running-3-validator-network.md).

## Project Structure

```
crates/
├── sequencer/     # Main binary
├── consensus/     # PoA block production (commonware-consensus)
├── celestia/      # Celestia DA client + finality tracking
├── execution/     # Engine API client for reth
└── types/         # Shared types
```

## For Builders

This is piping, not policy.

## License

MIT OR Apache-2.0
