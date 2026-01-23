# RKB (Rauh-Konsens Begriff)

A Celestia-native PoA EVM sequencer. Minimal coordination layer wiring known components.

```
Reth      → EVM execution, mempool, block building
Simplex   → BFT consensus (commonware-consensus)
Celestia  → Data availability, firm finality
```

## Philosophy: Less Code, More Leverage

This project maintains minimal code by delegating to proven systems:

| Responsibility       | Delegated To         | Our Code          |
| -------------------- | -------------------- | ----------------- |
| Transaction ordering | reth mempool         | 0 lines           |
| Block building       | reth Engine API      | ~50 lines         |
| EVM execution        | reth                 | 0 lines           |
| BFT consensus        | commonware-consensus | ~200 lines glue   |
| Data availability    | Celestia             | ~300 lines client |
| P2P networking       | commonware-p2p       | ~100 lines config |

**Total: ~3,000 lines of Rust** orchestrating components with millions of lines of battle-tested code.

## Quick Start

```bash
# Build
cargo build --release

# Run 3-validator network
cd docker
./scripts/generate-keys.sh
# Copy config.example.toml → config.toml for each validator
# Fill in your Celestia endpoints
docker compose up -d --build

# Send transactions to reth (any validator)
cast send --rpc-url http://localhost:8545 ...
```

See [Running a 3-Validator Network](docs/running-3-validator-network.md) for detailed setup.

## Architecture

```
User Transactions ──► reth mempool ──► reth builds block
                                              │
                                              ▼
                      ┌───────────────────────────────────────┐
                      │         Simplex BFT Consensus         │
                      │  (commonware-consensus, ~200ms/block) │
                      └───────────────────────────────────────┘
                                              │
                      ┌───────────────────────┼───────────────────────┐
                      ▼                       ▼                       ▼
               reth executes           reth executes           reth executes
               (validator 0)           (validator 1)           (validator 2)
                      │                       │                       │
                      └───────────────────────┼───────────────────────┘
                                              │
                                              ▼
                                    ┌─────────────────┐
                                    │    Celestia     │
                                    │  (firm finality)│
                                    └─────────────────┘
```

**Key insight**: Users send transactions directly to reth. The sequencer has no mempool.

## Finality Model

| Level    | Trigger                 | Latency | Guarantee                |
| -------- | ----------------------- | ------- | ------------------------ |
| **Soft** | 2/3 BFT notarization    | ~200ms  | PoA validators agreed    |
| **Firm** | Celestia blob inclusion | ~6s     | Data availability proven |

## Project Structure

```
crates/
├── sequencer/     # Main binary, CLI, orchestration
├── consensus/     # Simplex BFT integration (Application trait)
├── celestia/      # DA client, blob submission, finality tracking
├── execution/     # reth Engine API client
└── types/         # Shared types, configuration
```

## How It Works

1. **User sends tx** to reth via `eth_sendRawTransaction`
2. **Leader validator** calls `engine_forkchoiceUpdatedV3` (start building)
3. **reth builds block** from its mempool, orders by gas price
4. **Leader gets block** via `engine_getPayloadV3`
5. **Simplex consensus** reaches agreement (~200ms)
6. **All validators execute** via `engine_newPayloadV3`
7. **Leader submits** to Celestia (batched every 5s)
8. **Celestia finalizes** (~6s for inclusion)

We write none of the complex parts. reth handles EVM. Celestia handles DA. commonware handles BFT.

## Documentation

| Document                                                           | Description                                      |
| ------------------------------------------------------------------ | ------------------------------------------------ |
| [CLAUDE.md](CLAUDE.md)                                             | AI assistant context, crate map, troubleshooting |
| [Architecture](docs/architecture.md)                               | Data flow diagrams, design decisions             |
| [Onboarding](docs/onboarding.md)                                   | Sequence diagrams, component responsibilities    |
| [Running 3-Validator Network](docs/running-3-validator-network.md) | Docker setup guide                               |
| [Docker README](docker/README.md)                                  | Quick start for local testing                    |

## License

MIT OR Apache-2.0
