# Onboarding: Understanding the Codebase

This document explains what you're maintaining, what you're not, and where the actual complexity lives.

---

## Table of Contents

- [The Big Picture](#the-big-picture)
- [What You Maintain vs What You Don't](#what-you-maintain-vs-what-you-dont)
- [Lines of Code Breakdown](#lines-of-code-breakdown)
- [Sequence Diagrams](#sequence-diagrams)
  - [Block Production (Vanilla Ethereum)](#block-production-vanilla-ethereum)
  - [Celestia Submission](#celestia-submission)
  - [Finality Tracking](#finality-tracking)
  - [Full Transaction Lifecycle](#full-transaction-lifecycle)
- [Component Responsibilities](#component-responsibilities)
- [Where to Look When Things Break](#where-to-look-when-things-break)

---

## The Big Picture

This repo is **plumbing**. It connects three production-grade systems:

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│   USER TX ──► RETH          YOUR CODE         CELESTIA          │
│              (mempool)  ──►  (plumbing)   ──►  (DA)             │
│              (builder)           │                               │
│                                  ▼                               │
│                            COMMONWARE                            │
│                            (consensus)                           │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Key insight:** Users send transactions directly to reth. We don't maintain a mempool.

**Your job:** Keep the pipes connected. The heavy lifting happens in libraries you don't maintain.

---

## What You Maintain vs What You Don't

| Component | Maintainer | Your Responsibility |
|-----------|------------|---------------------|
| **Reth mempool** | Paradigm | 0 lines (they handle it) |
| **Reth block building** | Paradigm | ~50 lines (call Engine API) |
| **Reth EVM execution** | Paradigm | 0 lines (they handle it) |
| **Commonware consensus** | Commonware team | ~200 lines glue |
| **Commonware P2P** | Commonware team | ~100 lines config |
| **Celestia/Lumina** | Celestia team | ~300 lines client |
| **This repo** | You | ~5,000 lines of glue |

**Translation:** When consensus breaks, it's probably Commonware. When execution breaks, it's probably Reth. When DA breaks, it's probably Celestia. When the glue breaks, it's your code.

---

## Lines of Code Breakdown

### What You Actually Wrote

| Module | Purpose | Approx LoC | Complexity |
|--------|---------|------------|------------|
| `crates/consensus/` | Wrap Commonware, Application trait | ~500 | Low - callbacks |
| `crates/execution/` | Engine API client, BlockBuilder | ~500 | Medium - state tracking |
| `crates/celestia/` | Blob submission, finality tracking | ~800 | Medium - async coordination |
| `crates/types/` | Block, transaction, config structs | ~300 | Low - data definitions |
| `crates/sequencer/` | CLI, startup, main loop | ~500 | Low - orchestration |
| **Total** | | **~2,600** | |

### What You Didn't Write (But Use)

| Dependency | Approx LoC | You Touch |
|------------|------------|-----------|
| Commonware consensus | ~15,000 | 0% |
| Commonware p2p | ~10,000 | 0% |
| Reth | ~200,000+ | 0% |
| Lumina | ~20,000+ | 0% |
| Alloy types | ~50,000+ | 0% |

**The ratio:** You maintain ~2,600 lines. You depend on ~300,000+ lines. That's the point.

---

## Sequence Diagrams

### Block Production (Vanilla Ethereum)

**This is the key architectural insight.** We don't build blocks—reth does.

```
┌──────┐     ┌──────┐     ┌───────────┐     ┌───────────┐
│ User │     │ Reth │     │ Sequencer │     │Commonware │
└──┬───┘     └──┬───┘     └─────┬─────┘     └─────┬─────┘
   │            │               │                 │
   │ eth_sendRawTransaction     │                 │
   │───────────>│               │                 │
   │            │               │                 │
   │            │ (tx in mempool)                 │
   │            │               │                 │
   │            │               │  (Leader elected)
   │            │               │<────────────────│
   │            │               │                 │
   │            │  forkchoiceUpdatedV3           │
   │            │  + PayloadAttributes           │
   │            │<──────────────│                 │
   │            │               │                 │
   │            │  PayloadId    │                 │
   │            │──────────────>│                 │
   │            │               │                 │
   │            │  (reth builds block            │
   │            │   from mempool)                │
   │            │               │                 │
   │            │  getPayloadV3 │                 │
   │            │<──────────────│                 │
   │            │               │                 │
   │            │  BuiltPayload │                 │
   │            │  (block_hash, │                 │
   │            │   state_root, │                 │
   │            │   txs...)     │                 │
   │            │──────────────>│                 │
   │            │               │                 │
   │            │               │  Propose block  │
   │            │               │  (reth's hash)  │
   │            │               │────────────────>│
   │            │               │                 │
   │            │               │  2/3 votes      │
   │            │               │  (notarized)    │
   │            │               │<────────────────│
   │            │               │                 │
   │            │  newPayloadV3 │ (all validators)
   │            │<──────────────│                 │
   │            │               │                 │
   │            │  VALID        │                 │
   │            │──────────────>│                 │
   │            │               │                 │
   │ Tx confirmed (soft finality, ~200ms)        │
   │<───────────│               │                 │
```

**Your code:** The Sequencer column. You call reth's Engine API to:
1. Start building (`forkchoiceUpdatedV3` with `PayloadAttributes`)
2. Get the built block (`getPayloadV3`)
3. Execute agreed blocks (`newPayloadV3`)

**You don't write:**
- Mempool logic (reth)
- Transaction ordering (reth)
- Gas accounting (reth)
- State root computation (reth)
- BFT consensus (Commonware)

---

### Celestia Submission

```
┌───────────┐     ┌─────────────┐     ┌─────────┐     ┌──────────┐
│ Sequencer │     │ BatchBuffer │     │ Lumina  │     │ Celestia │
└─────┬─────┘     └──────┬──────┘     └────┬────┘     └────┬─────┘
      │                  │                 │               │
      │  Block finalized │                 │               │
      │  (soft)          │                 │               │
      │─────────────────>│                 │               │
      │                  │                 │               │
      │                  │  (accumulate    │               │
      │                  │   for ~5s)      │               │
      │                  │                 │               │
      │  More blocks...  │                 │               │
      │─────────────────>│                 │               │
      │                  │                 │               │
      │                  │  Batch ready    │               │
      │                  │  (timeout/size) │               │
      │                  │                 │               │
      │                  │  submit_blocks()│               │
      │                  │────────────────>│               │
      │                  │                 │               │
      │                  │                 │  PayForBlobs  │
      │                  │                 │──────────────>│
      │                  │                 │               │
      │                  │                 │  TxInfo       │
      │                  │                 │<──────────────│
      │                  │                 │               │
      │                  │  BlobSubmission │               │
      │                  │<────────────────│               │
      │                  │                 │               │
      │  Batch submitted │                 │               │
      │<─────────────────│                 │               │
      │  (DA finality    │                 │               │
      │   in ~6s)        │                 │               │
```

**Your code:** `BatchBuffer` and the glue. Lumina handles the actual Celestia protocol.

---

### Finality Tracking

```
┌───────────┐     ┌─────────────────┐     ┌─────────┐     ┌──────┐
│ Sequencer │     │ FinalityTracker │     │ Lumina  │     │ Reth │
└─────┬─────┘     └────────┬────────┘     └────┬────┘     └──┬───┘
      │                    │                   │             │
      │                    │  Subscribe headers│             │
      │                    │──────────────────>│             │
      │                    │                   │             │
      │  Track batch       │                   │             │
      │  (after submit)    │                   │             │
      │───────────────────>│                   │             │
      │                    │                   │             │
      │                    │  pending[batch_id]│             │
      │                    │  = celestia_height│             │
      │                    │                   │             │
      │                    │                   │  (Celestia  │
      │                    │                   │   produces  │
      │                    │                   │   blocks)   │
      │                    │                   │             │
      │                    │  New header at H  │             │
      │                    │<──────────────────│             │
      │                    │                   │             │
      │                    │  Check: any       │             │
      │                    │  pending ≤ H?     │             │
      │                    │                   │             │
      │                    │  Yes → batch final│             │
      │                    │                   │             │
      │  FirmConfirmation  │                   │             │
      │<───────────────────│                   │             │
      │                    │                   │             │
      │  forkchoiceUpdatedV3                   │             │
      │  (update FINALIZED)                    │             │
      │────────────────────────────────────────────────────>│
      │                    │                   │             │
```

**Your code:** `FinalityTracker`. Lumina tells you when Celestia headers arrive. You match them to pending batches.

---

### Full Transaction Lifecycle

```
┌──────┐  ┌──────┐  ┌───────────┐  ┌───────────┐  ┌─────────┐  ┌──────────┐
│ User │  │ Reth │  │ Sequencer │  │Commonware │  │ Lumina  │  │ Celestia │
└──┬───┘  └──┬───┘  └─────┬─────┘  └─────┬─────┘  └────┬────┘  └────┬─────┘
   │         │            │              │             │            │
   │ 1. Send │            │              │             │            │
   │────────>│            │              │             │            │
   │         │            │              │             │            │
   │         │ (mempool)  │              │             │            │
   │         │            │              │             │            │
   │         │            │ 2. Leader    │             │            │
   │         │            │    elected   │             │            │
   │         │            │<─────────────│             │            │
   │         │            │              │             │            │
   │         │ 3. Start   │              │             │            │
   │         │    build   │              │             │            │
   │         │<───────────│              │             │            │
   │         │            │              │             │            │
   │         │ 4. Payload │              │             │            │
   │         │───────────>│              │             │            │
   │         │            │              │             │            │
   │         │            │ 5. Propose   │             │            │
   │         │            │─────────────>│             │            │
   │         │            │              │             │            │
   │         │            │ 6. Notarize  │             │            │
   │         │            │<─────────────│             │            │
   │         │            │              │             │            │
   │         │ 7. Execute │              │             │            │
   │         │<───────────│ (all nodes)  │             │            │
   │         │            │              │             │            │
   │ 8. SOFT ✓│           │              │             │            │
   │<────────│ (~200ms)   │              │             │            │
   │         │            │              │             │            │
   │         │            │ 9. Batch     │             │            │
   │         │            │─────────────────────────────────────────>│
   │         │            │              │             │            │
   │         │            │              │  10. Header │            │
   │         │            │              │<────────────│            │
   │         │            │              │             │            │
   │         │            │ 11. Update   │             │            │
   │         │            │    FINALIZED │             │            │
   │         │<───────────│              │             │            │
   │         │            │              │             │            │
   │12. FIRM ✓│           │              │             │            │
   │<────────│ (~6s)      │              │             │            │
```

**Your code:** Steps 2-7 (consensus↔reth glue), 9 (batching), 10-11 (finality tracking).

**Not your code:** Step 1 (reth mempool), Step 8 (reth state), Celestia protocol.

---

## Component Responsibilities

### `crates/consensus/application.rs`

**What it does:**
- Implement `Automaton` trait for Commonware
- Call `execution.start_building()` to trigger reth block building
- Call `execution.get_payload()` to retrieve built block
- Return reth's authoritative block hash to consensus

**What it doesn't do:**
- Build blocks (reth does this)
- Order transactions (reth does this)
- Compute state roots (reth does this)
- BFT logic (Commonware)

---

### `crates/execution/client.rs`

**What it does:**
- `start_building()` - Call `forkchoiceUpdatedV3` with `PayloadAttributes`
- `get_payload()` - Call `getPayloadV3` to retrieve built block
- `execute_block()` - Call `newPayloadV3` to execute consensus blocks
- `update_forkchoice()` - Track HEAD/SAFE/FINALIZED

**What it doesn't do:**
- EVM execution (Reth)
- State storage (Reth)
- Mempool management (Reth)
- Transaction validation (Reth)

---

### `crates/celestia/client.rs`

**What it does:**
- `submit_blocks()` - Batch blocks into single PayForBlobs tx
- `run_finality_tracker()` - Subscribe to Celestia headers
- Match headers to pending submissions
- Emit `FinalityConfirmation` events

**What it doesn't do:**
- Celestia protocol (Lumina)
- Data availability sampling (Lumina)
- Light client sync (Lumina)

---

### `crates/sequencer/simplex_sequencer.rs`

**What it does:**
- Initialize reth connection, get genesis hash
- Initialize Celestia connection
- Spawn batch submission task
- Wire finalization callback
- Handle Celestia finality → reth forkchoice updates

**What it doesn't do:**
- Any blockchain logic (delegates everything)

---

## Where to Look When Things Break

### Symptoms → Likely Cause

| Symptom | First Check | Likely Cause |
|---------|-------------|--------------|
| "Invalid forkchoice state" | Genesis hash | Wrong initial hash |
| "No payload ID" | forkchoiceUpdated call | Missing init_forkchoice |
| Blocks not producing | Commonware logs | Consensus issue |
| Execution errors | Reth logs | Invalid payload |
| Blobs not landing | Celestia explorer | Lumina connection or gas |
| Finality stuck | FinalityTracker state | Celestia sync lag |

### Debug Sequence

```
1. Is reth accepting forkchoice?
   └─ No  → Check genesis hash, init_forkchoice called
   └─ Yes → Continue

2. Is reth returning payloads?
   └─ No  → Check PayloadAttributes, timestamp
   └─ Yes → Continue

3. Is Commonware reaching consensus?
   └─ No  → Check validator config, p2p connectivity
   └─ Yes → Continue

4. Are blobs landing on Celestia?
   └─ No  → Check Lumina connection, namespace, gas
   └─ Yes → Continue

5. Is finality updating?
   └─ No  → Check FinalityTracker, header subscription
   └─ Yes → System healthy
```

---

## The Maintenance Mental Model

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  COMPLEXITY YOU INHERIT          COMPLEXITY YOU OWN             │
│  (don't touch)                   (your problem)                 │
│                                                                  │
│  • Mempool                       • Calling Engine API           │
│  • Transaction ordering          • Config management            │
│  • EVM execution                 • Batch timing                 │
│  • State tries                   • Finality state machine       │
│  • BFT consensus                 • Error handling               │
│  • P2P networking                • Logging/metrics              │
│  • Celestia protocol             • Blob format                  │
│  • DAS                           • Genesis hash init            │
│  • ZK circuits (future)                                         │
│                                                                  │
│  ~300,000 lines                  ~2,600 lines                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

When something breaks, first ask: "Is this my 2,600 lines or their 300,000?"

Usually it's a configuration issue, not a code issue.

---

## Summary

- **You maintain plumbing**, not a blockchain
- **Users send tx to reth**, not to you
- **Reth builds blocks**, you just ask for them
- **~2,600 lines** of glue code
- **Sequence diagrams** show exactly where your code sits
- **When debugging**, check the boundaries first
- **Most complexity** lives in dependencies you don't touch

The goal is to keep this codebase boring. Boring is good. Boring means it works.
