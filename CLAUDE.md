# CLAUDE.md

RKB is a Celestia-native PoA EVM sequencer. You maintain ~1,400 lines of plumbing between Commonware (consensus), Reth (execution), and Celestia (DA).

---

## The Mental Model

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  COMPLEXITY YOU INHERIT          COMPLEXITY YOU OWN             │
│  (don't touch)                   (your problem)                 │
│                                                                  │
│  • BFT consensus (~15k LoC)      • Wiring components (~1,400)   │
│  • P2P networking (~10k LoC)     • Config management            │
│  • EVM execution (~200k LoC)     • Batch timing                 │
│  • Celestia protocol (~20k LoC)  • Finality state machine       │
│                                                                  │
│  ~300,000+ lines                 ~1,400 lines                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

When something breaks, first ask: **"Is this my 1,400 lines or their 300,000?"**

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
- Soft (~200ms): PoA consensus
- Firm (~6s): Celestia DA inclusion

---

## Crate Map

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `sequencer` | CLI + main loop | `main.rs`, `cli.rs`, `simplex_sequencer.rs` |
| `consensus` | Commonware wrapper | `simplex.rs`, `application.rs`, `runtime.rs` |
| `execution` | Reth Engine API | `client.rs`, `forkchoice.rs` |
| `celestia` | Blob submission + finality | `client.rs`, `finality.rs` |
| `types` | Shared structs | `block.rs`, `config.rs`, `transaction.rs` |

---

## Common Tasks

### Modify consensus behavior
→ `crates/consensus/src/simplex.rs` (Simplex BFT integration)
→ `crates/consensus/src/application.rs` (CertifiableAutomaton impl)

### Fix execution/Engine API issues
→ `crates/execution/src/client.rs` (newPayload, forkchoiceUpdated calls)
→ `crates/execution/src/forkchoice.rs` (HEAD/SAFE/FINALIZED tracking)

### Debug Celestia submission
→ `crates/celestia/src/client.rs` (blob submission via Lumina)
→ `crates/celestia/src/finality.rs` (header subscription, finality tracking)

### Add/modify config options
→ `crates/types/src/config.rs` (all config structs)

### Add new shared types
→ `crates/types/src/block.rs` or `transaction.rs`

---

## Troubleshooting

| Symptom | First Check | Likely Cause | Fix |
|---------|-------------|--------------|-----|
| Blocks not producing | Commonware logs | Validator config, P2P | Check `consensus/src/network.rs` |
| Engine API errors | Reth logs | Invalid payload, JWT | Check `execution/src/client.rs` |
| Blobs not landing | Celestia explorer | Gas, namespace | Check `celestia/src/client.rs` |
| Finality stuck | FinalityTracker | Celestia sync lag | Check `celestia/src/finality.rs` |
| State root mismatch | Compare roots | Bug in glue code | Diff execution vs Celestia |

### Debug Sequence

```
1. Commonware producing blocks?
   └─ No → validator config, p2p connectivity
   └─ Yes → continue

2. Reth accepting payloads?
   └─ No → Engine API response, payload format
   └─ Yes → continue

3. Blobs landing on Celestia?
   └─ No → Lumina connection, namespace, gas
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
```

---

## What NOT to Touch

These are external dependencies. If they break, it's not your code:

| If this breaks... | Check here... |
|-------------------|---------------|
| Consensus logic | Commonware repo |
| EVM execution | Reth repo |
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
- `eyre::Result` for context-heavy errors
- `.context("message")` for error chains

### Logging
- `tracing` macros: `info!`, `warn!`, `error!`, `debug!`
- Structured fields: `info!(block = %height, "produced")`
- `RUST_LOG=debug` for verbose output

### Config
- TOML files for configuration
- Serde deserialization
- Separate `config.rs` per crate

---

## Deep Dives

For detailed documentation, see:

| Topic | File |
|-------|------|
| Full architecture + sequence diagrams | `docs/onboarding.md` |
| Data flow + finality model | `docs/architecture.md` |
| Multi-validator setup | `docs/running-3-validator-network.md` |
| Future ZK verification | `docs/roadmap-zk-verification.md` |

---

## Quick Reference

```
crates/
├── sequencer/     # Binary entry point, CLI
├── consensus/     # Commonware PoA/Simplex wrapper
├── execution/     # Reth Engine API client
├── celestia/      # Lumina blob submission + finality
└── types/         # Shared Block, Transaction, Config

docs/
├── onboarding.md           # Start here for deep dive
├── architecture.md         # Data flow diagrams
└── roadmap-zk-verification.md  # Future ZK work
```

---

**Remember:** This is plumbing, not a blockchain. Keep it boring. Boring means it works.
