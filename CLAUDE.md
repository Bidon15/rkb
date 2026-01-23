# CLAUDE.md

RKB (Rauh-Konsens Begriff) is a Celestia-native PoA EVM sequencer. You maintain ~6,400 lines of plumbing between Commonware (consensus), Reth (execution), and Celestia (DA).

---

## The Mental Model

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  COMPLEXITY YOU INHERIT          COMPLEXITY YOU OWN             │
│  (don't touch)                   (your problem)                 │
│                                                                  │
│  • BFT consensus (~15k LoC)      • Wiring components (~6,400)   │
│  • P2P networking (~10k LoC)     • Config management            │
│  • EVM execution (~200k LoC)     • Batch timing                 │
│  • Celestia protocol (~20k LoC)  • Finality state machine       │
│                                                                  │
│  ~300,000+ lines                 ~6,400 lines                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

When something breaks, first ask: **"Is this my 6,400 lines or their 300,000?"**

---

## Architecture

```
┌───────────┐     ┌───────────┐     ┌───────────┐
│Commonware │ ──► │ YOUR CODE │ ──► │   Reth    │
│(consensus)│     │ (plumbing)│     │(execution)│
└───────────┘     └─────┬─────┘     └───────────┘
                        │
                        ▼
                  ┌───────────┐
                  │ Celestia  │
                  │   (DA)    │
                  └───────────┘
```

**Data Flow:** `Transactions → Consensus → Execution → Celestia`

**Finality:**
- Soft (~200ms): PoA consensus (2/3 BFT notarization)
- Firm (~6s): Celestia DA inclusion

---

## Crate Map

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `sequencer` | CLI + orchestration | `cli.rs`, `simplex_sequencer.rs` |
| `consensus` | Simplex BFT wrapper | `application.rs`, `runtime.rs`, `leader.rs` |
| `execution` | Reth Engine API | `client.rs`, `forkchoice.rs`, `jwt.rs` |
| `celestia` | Blob submission + finality | `client.rs`, `finality.rs` |
| `types` | Shared structs | `block.rs`, `config.rs`, `transaction.rs` |

---

## Common Tasks

### Modify consensus behavior
→ `crates/consensus/src/application.rs` (CertifiableAutomaton impl, block building)
→ `crates/consensus/src/runtime.rs` (Simplex runtime setup, finalization callback)

### Fix execution/Engine API issues
→ `crates/execution/src/client.rs` (newPayload, forkchoiceUpdated, getPayload calls)
→ `crates/execution/src/forkchoice.rs` (HEAD/SAFE/FINALIZED tracking)
→ `crates/execution/src/jwt.rs` (JWT token generation for Engine API auth)

### Debug Celestia submission
→ `crates/celestia/src/client.rs` (blob submission, PayForBlobs)
→ `crates/celestia/src/finality.rs` (header subscription, finality tracking)

### Add/modify config options
→ `crates/types/src/config.rs` (all config structs, defaults)

### Main orchestration loop
→ `crates/sequencer/src/simplex_sequencer.rs` (startup, sync, runtime wiring)

---

## Troubleshooting

| Symptom | First Check | Likely Cause | Fix |
|---------|-------------|--------------|-----|
| Blocks not producing | Commonware logs | Validator config, P2P | Check network/peers config |
| `-38002` Invalid forkchoice | executed_blocks set | Block not on canonical chain | Check `handle_finality_confirmation` |
| `-38003` Invalid payload | timestamp | `block.timestamp <= parent` | Check `compute_block_timestamp` |
| JWT auth failures | token expiry | `iat` claim > 60s old | Check `jwt.rs` per-request tokens |
| Blobs not landing | Celestia explorer | Gas, namespace, account | Check `celestia/src/client.rs` |
| Duplicate blocks | application state | Multiple proposals same height | Check `last_proposed_height` in application.rs |

### Debug Sequence

```
1. Commonware producing blocks?
   └─ No → validator config, p2p connectivity
   └─ Yes → continue

2. Reth accepting payloads?
   └─ No → Engine API response, payload format, JWT
   └─ Yes → continue

3. Blobs landing on Celestia?
   └─ No → Lumina connection, namespace, gas, account funding
   └─ Yes → continue

4. Finality updating?
   └─ No → FinalityTracker, header subscription
   └─ Yes → system healthy
```

---

## Key Commands

```bash
# Check everything compiles
cargo check --workspace

# Run lints
cargo clippy --workspace

# Run tests
cargo test --workspace

# Format check
cargo fmt --check

# Build release
cargo build --release

# Run locally
./target/release/sequencer run -c config.toml

# Get Celestia address from key
./target/release/sequencer celestia-address --key path/to/key
```

---

## What NOT to Touch

These are external dependencies. If they break, it's not your code:

| If this breaks... | Check here... |
|-------------------|---------------|
| Consensus logic | commonware-consensus repo |
| EVM execution | reth repo |
| Celestia protocol | Lumina/Celestia repos |
| Engine API types | Alloy repo |

**Rule:** Don't modify external crate internals. Use their public APIs.

---

## Conventions

### Async
- All I/O uses `tokio` runtime
- `#[tokio::main]` in main.rs
- `async fn` for anything that waits

### Error Handling
- `thiserror` for custom error types
- Per-crate `Result<T>` type aliases
- `.context("message")` for error chains

### Logging
- `tracing` macros: `info!`, `warn!`, `error!`, `debug!`
- Structured fields: `info!(block_height, %block_hash, "executed")`
- `RUST_LOG=debug` for verbose output

### Config
- TOML files for configuration
- Serde deserialization
- Defaults in `types/src/config.rs::defaults` module

---

## Critical Safety Mechanisms

These prevent production failures:

| Mechanism | Location | Purpose |
|-----------|----------|---------|
| `last_proposed_height` | application.rs | Prevents duplicate block proposals at same height |
| Height check in `finalize()` | application.rs | Rejects stale block finalizations |
| `executed_blocks` set | simplex_sequencer.rs | Only finalize blocks we've actually executed |
| Per-request JWT | execution/client.rs | Fresh `iat` claim for each Engine API call |
| `now.max(parent_timestamp + 1)` | application.rs | Ensures valid block timestamps |

---

## Quick Reference

```
crates/
├── sequencer/     # Binary entry point, CLI, orchestration
├── consensus/     # Commonware Simplex BFT wrapper
├── execution/     # Reth Engine API client
├── celestia/      # Blob submission + finality tracking
└── types/         # Shared Block, Transaction, Config

docs/
├── architecture.md              # Data flow diagrams
├── onboarding.md                # Sequence diagrams, deep dive
└── running-3-validator-network.md  # Docker setup

docker/
├── config/validator-*/config.example.toml  # Template configs
├── keys/                        # Generated keys (gitignored)
├── scripts/generate-keys.sh     # Key generation
└── docker-compose.yml           # 3-validator network
```

---

**Remember:** This is plumbing, not a blockchain. Keep it boring. Boring means it works.
