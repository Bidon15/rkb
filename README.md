# RKB: Reth-Kernel-Blob

A Celestia-native PoA EVM sequencer. Minimal coordination layer wiring battle-tested components.

```
R = Reth      (EVM execution, mempool, block building)
K = Kernel    (commonware-consensus Simplex BFT)
B = Blob      (Celestia data availability)
```

## Philosophy: Less Code, More Leverage

This project deliberately maintains minimal code by delegating to proven systems:

| Responsibility | Delegated To | Our Code |
|---------------|--------------|----------|
| Transaction ordering | reth mempool | 0 lines |
| Block building | reth Engine API | ~50 lines |
| EVM execution | reth | 0 lines |
| BFT consensus | commonware-consensus | ~200 lines glue |
| Data availability | Celestia | ~300 lines client |
| P2P networking | commonware-p2p | ~100 lines config |

**Total: ~5,000 lines of Rust** orchestrating components with millions of lines of battle-tested code.

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

**Key insight**: Users send transactions directly to reth. We don't maintain a mempool.

## What This Gives You

- **Instant soft finality** (~200ms) via PoA consensus
- **Firm finality** (~6s) via Celestia inclusion
- **Full EVM compatibility** (vanilla reth)
- **No mempool code** (reth handles it)
- **No block building code** (reth handles it)
- **No transaction ordering logic** (reth handles it)

## Quick Start

```bash
# Build
cargo build --release

# Run 3-validator network (see docker/README.md)
cd docker
./scripts/generate-keys.sh
docker compose up -d --build

# Send transactions to reth (any validator)
cast send --rpc-url http://localhost:8545 ...
```

For detailed setup, see [Running a 3-Validator Network](docs/running-3-validator-network.md).

## Project Structure

```
crates/
├── sequencer/     # Main binary (~1,500 lines)
├── consensus/     # Simplex BFT glue (~1,200 lines)
├── celestia/      # DA client + finality (~800 lines)
├── execution/     # Engine API client (~400 lines)
└── types/         # Shared types (~300 lines)
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

## Design Decisions

### Why vanilla reth block building?

Alternative approaches and their costs:

| Approach | Lines of Code | Maintenance Burden |
|----------|--------------|-------------------|
| Custom mempool + ordering | ~3,000+ | Transaction validation, nonce tracking, replacement logic |
| Modified reth (op-reth style) | ~10,000+ | Fork maintenance, merge conflicts |
| **Vanilla Engine API** | **~50** | **None** |

### Why no sequencer mempool?

- reth already has a production-grade mempool
- reth already handles nonce ordering
- reth already handles gas price priority
- Why reimplement?

### Trade-offs accepted

- Transaction ordering controlled by reth (gas price priority)
- Leader can theoretically censor (but PoA validators are trusted)
- No custom ordering schemes (acceptable for PoA)

## For Builders

This is infrastructure, not policy. Build your application logic elsewhere.

## License

MIT OR Apache-2.0
