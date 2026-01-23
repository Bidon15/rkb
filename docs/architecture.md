# RKB Architecture

## Design Philosophy: Minimal Glue Code

This sequencer is deliberately thin. We orchestrate existing systems rather than reimplementing them.

```
What we build:     ~5,000 lines of Rust (glue)
What we leverage:  Millions of lines (reth, Celestia, commonware)
```

## Component Responsibilities

| Component | What It Does | What We Don't Do |
|-----------|--------------|------------------|
| **reth** | Mempool, block building, EVM execution | We don't maintain transaction pools |
| **commonware-consensus** | BFT agreement, leader election | We don't implement consensus protocols |
| **Celestia** | Data availability, finality | We don't run a DA layer |
| **commonware-p2p** | Peer discovery, message routing | We don't implement networking |

## Transaction Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              USER TRANSACTION                                │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         reth JSON-RPC (eth_sendRawTransaction)              │
│                                                                              │
│   User sends tx to ANY reth instance. reth validates and adds to mempool.   │
│   We write: 0 lines                                                          │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         BLOCK BUILDING (Leader Only)                         │
│                                                                              │
│   1. Leader calls engine_forkchoiceUpdatedV3 with PayloadAttributes         │
│   2. reth starts building block from its mempool                            │
│   3. Leader calls engine_getPayloadV3 to retrieve built block               │
│                                                                              │
│   We write: ~50 lines (two RPC calls)                                        │
│   reth handles: tx ordering, gas accounting, state transitions               │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         SIMPLEX BFT CONSENSUS                                │
│                                                                              │
│   1. Leader proposes block (reth-built, with correct hash)                  │
│   2. Validators vote                                                         │
│   3. 2/3+ votes = notarized (SOFT FINALITY, ~200ms)                         │
│                                                                              │
│   We write: ~200 lines (Application trait impl)                              │
│   commonware handles: voting, view changes, byzantine tolerance              │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         BLOCK EXECUTION (All Validators)                     │
│                                                                              │
│   Each validator calls engine_newPayloadV3 + forkchoiceUpdated              │
│   reth executes transactions and updates state                              │
│                                                                              │
│   We write: ~100 lines (RPC calls + forkchoice tracking)                     │
│   reth handles: EVM execution, state storage, receipt generation             │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         CELESTIA SUBMISSION (Leader Only)                    │
│                                                                              │
│   Leader batches blocks and submits via PayForBlobs                         │
│   Tracks inclusion for finality confirmation                                 │
│                                                                              │
│   We write: ~300 lines (batching + finality tracking)                        │
│   Celestia handles: consensus, data availability proofs                      │
└─────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         FIRM FINALITY (~6s)                                  │
│                                                                              │
│   Celestia includes blob → we update forkchoice.finalized                   │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Finality Model

| Level | Trigger | Latency | Guarantee |
|-------|---------|---------|-----------|
| **Soft** | 2/3 BFT notarization | ~200ms | PoA validators agreed |
| **Firm** | Celestia blob inclusion | ~6s | Data availability proven |

Maps to Ethereum forkchoice:

```
HEAD      = Latest soft-confirmed block (BFT notarized)
SAFE      = Same as HEAD (PoA consensus is authoritative)
FINALIZED = Latest Celestia-finalized block
```

## Why Vanilla reth?

### The Block Hash Problem

EVM block hashes include fields computed during execution:
- `state_root` - Merkle root of account states
- `receipts_root` - Merkle root of transaction receipts
- `logs_bloom` - Bloom filter of logs

**If we build blocks ourselves**, we can't compute the correct hash until after execution. This creates a chicken-and-egg problem.

### Alternative Approaches

| Approach | Problem |
|----------|---------|
| **Two-hash system** | Debugging nightmare, confusing for users |
| **Custom reth fork** | Maintenance burden, merge conflicts |
| **Pre-execution hash** | Incompatible with standard tooling |

### Our Solution

Let reth build blocks. It computes the correct hash. We just orchestrate.

```
Leader: "reth, build me a block"
reth:   "here's block 0xabc123..."
Leader: "validators, agree on 0xabc123"
All:    "agreed, executing 0xabc123"
```

## Code Organization

```
crates/
├── sequencer/        # Binary, config, main loop
│   └── simplex_sequencer.rs   # Orchestration (~500 lines)
│
├── consensus/        # Simplex BFT integration
│   ├── application.rs   # Block building via reth (~200 lines)
│   └── runtime.rs       # commonware runtime setup (~300 lines)
│
├── execution/        # reth Engine API client
│   ├── client.rs     # newPayload, forkchoiceUpdated, getPayload (~350 lines)
│   └── forkchoice.rs # HEAD/SAFE/FINALIZED tracking (~50 lines)
│
├── celestia/         # Celestia DA client
│   ├── client.rs     # Blob submission (~400 lines)
│   └── finality.rs   # Inclusion tracking (~200 lines)
│
└── types/            # Shared types
    └── ~300 lines
```

## What We Explicitly Don't Do

- **No custom mempool** - reth has one
- **No transaction validation** - reth does it
- **No nonce management** - reth handles it
- **No gas price ordering** - reth handles it
- **No EVM implementation** - reth is the EVM
- **No consensus protocol** - commonware implements Simplex
- **No P2P networking** - commonware handles it
- **No DA consensus** - Celestia handles it

## Trade-offs

### Accepted

- Transaction ordering by gas price (reth default)
- Leader controls block contents (acceptable for trusted PoA)
- No custom sequencing logic (use reth's mempool behavior)

### Rejected Complexity

- Custom transaction ordering schemes
- MEV protection (not needed for PoA with trusted validators)
- Multiple execution clients (reth only)
- Custom block building pipelines
