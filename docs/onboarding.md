# Onboarding: Understanding the Codebase

This document explains what you're maintaining, what you're not, and where the actual complexity lives.

---

## Table of Contents

- [The Big Picture](#the-big-picture)
- [What You Maintain vs What You Don't](#what-you-maintain-vs-what-you-dont)
- [Lines of Code Breakdown](#lines-of-code-breakdown)
- [Sequence Diagrams](#sequence-diagrams)
  - [Block Production](#block-production)
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
│   COMMONWARE          YOUR CODE           RETH                  │
│   (consensus)    ──→  (plumbing)    ──→   (execution)           │
│                           │                                      │
│                           ▼                                      │
│                       CELESTIA                                   │
│                       (DA)                                       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Your job:** Keep the pipes connected. The heavy lifting happens in libraries you don't maintain.

---

## What You Maintain vs What You Don't

| Component | Maintainer | Your Responsibility |
|-----------|------------|---------------------|
| **Commonware** | Commonware team | Use their APIs correctly |
| **Reth** | Paradigm | Call Engine API correctly |
| **Celestia/Lumina** | Celestia team | Submit blobs correctly |
| **RSP** (future) | Succinct | Feed it block data |
| **This repo** | You | ~1500 lines of glue |

**Translation:** When consensus breaks, it's probably Commonware. When execution breaks, it's probably Reth. When DA breaks, it's probably Celestia. When the glue breaks, it's your code.

---

## Lines of Code Breakdown

### What You Actually Wrote

| Module | Purpose | Approx LoC | Complexity |
|--------|---------|------------|------------|
| `src/consensus/` | Wrap Commonware, manage validators | ~300 | Low - config and callbacks |
| `src/execution/` | Engine API client to Reth | ~400 | Medium - state tracking |
| `src/celestia/` | Blob submission, finality tracking | ~300 | Medium - async coordination |
| `src/types/` | Block, transaction, config structs | ~200 | Low - data definitions |
| `src/main.rs` | CLI, startup, main loop | ~200 | Low - orchestration |
| **Total** | | **~1400** | |

### What You Didn't Write (But Use)

| Dependency | Approx LoC | You Touch |
|------------|------------|-----------|
| Commonware consensus | ~15,000 | 0% |
| Commonware p2p | ~10,000 | 0% |
| Reth | ~200,000+ | 0% |
| Lumina | ~20,000+ | 0% |
| Alloy types | ~50,000+ | 0% |

**The ratio:** You maintain ~1,400 lines. You depend on ~300,000+ lines. That's the point.

---

## Sequence Diagrams

### Block Production

```
┌──────────┐     ┌───────────┐     ┌──────────┐     ┌──────┐
│  User    │     │ Sequencer │     │Commonware│     │ Reth │
└────┬─────┘     └─────┬─────┘     └────┬─────┘     └──┬───┘
     │                 │                │              │
     │  Submit Tx      │                │              │
     │────────────────>│                │              │
     │                 │                │              │
     │                 │  Add to mempool│              │
     │                 │───────────────>│              │
     │                 │                │              │
     │                 │                │  (PoA round) │
     │                 │                │──────────────│
     │                 │                │              │
     │                 │  Block ready   │              │
     │                 │<───────────────│              │
     │                 │                │              │
     │                 │  engine_newPayloadV3         │
     │                 │──────────────────────────────>│
     │                 │                │              │
     │                 │  PayloadStatus: VALID        │
     │                 │<──────────────────────────────│
     │                 │                │              │
     │                 │  engine_forkchoiceUpdatedV3  │
     │                 │──────────────────────────────>│
     │                 │                │              │
     │                 │  ForkchoiceState updated     │
     │                 │<──────────────────────────────│
     │                 │                │              │
     │  Tx confirmed   │                │              │
     │<────────────────│                │              │
     │  (soft finality)│                │              │
```

**Your code:** The middle column. You're calling Commonware and Reth APIs.

---

### Celestia Submission

```
┌───────────┐     ┌─────────────┐     ┌─────────┐     ┌──────────┐
│ Sequencer │     │ BatchBuffer │     │ Lumina  │     │ Celestia │
└─────┬─────┘     └──────┬──────┘     └────┬────┘     └────┬─────┘
      │                  │                 │               │
      │  Block executed  │                 │               │
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
      │                  │  Submit blob    │               │
      │                  │────────────────>│               │
      │                  │                 │               │
      │                  │                 │  blob.submit()│
      │                  │                 │──────────────>│
      │                  │                 │               │
      │                  │                 │  TxInfo       │
      │                  │                 │<──────────────│
      │                  │                 │               │
      │                  │  Commitment     │               │
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
      │  engine_forkchoiceUpdatedV3           │             │
      │  (update FINALIZED)                    │             │
      │────────────────────────────────────────────────────>│
      │                    │                   │             │
```

**Your code:** `FinalityTracker`. Lumina tells you when Celestia headers arrive. You match them to pending batches.

---

### Full Transaction Lifecycle

```
┌──────┐  ┌───────────┐  ┌───────────┐  ┌──────┐  ┌─────────┐  ┌──────────┐
│ User │  │ Sequencer │  │Commonware │  │ Reth │  │ Lumina  │  │ Celestia │
└──┬───┘  └─────┬─────┘  └─────┬─────┘  └──┬───┘  └────┬────┘  └────┬─────┘
   │            │              │           │           │            │
   │ 1. Submit  │              │           │           │            │
   │───────────>│              │           │           │            │
   │            │              │           │           │            │
   │            │ 2. Order     │           │           │            │
   │            │─────────────>│           │           │            │
   │            │              │           │           │            │
   │            │ 3. Block     │           │           │            │
   │            │<─────────────│           │           │            │
   │            │              │           │           │            │
   │            │ 4. Execute   │           │           │            │
   │            │─────────────────────────>│           │            │
   │            │              │           │           │            │
   │ 5. SOFT ✓  │              │           │           │            │
   │<───────────│ (~200ms)     │           │           │            │
   │            │              │           │           │            │
   │            │ 6. Batch     │           │           │            │
   │            │─────────────────────────────────────>│            │
   │            │              │           │           │            │
   │            │              │           │ 7. Submit │            │
   │            │              │           │           │───────────>│
   │            │              │           │           │            │
   │            │              │           │ 8. Commit │            │
   │            │              │           │<──────────│            │
   │            │              │           │           │            │
   │ 9. DA ✓    │              │           │           │            │
   │<───────────│ (~6s)        │           │           │            │
   │            │              │           │           │            │
   │            │              │           │10. Header │            │
   │            │              │           │<──────────│            │
   │            │              │           │           │            │
   │            │ 11. Update   │           │           │            │
   │            │    FINALIZED │           │           │            │
   │            │─────────────────────────>│           │            │
   │            │              │           │           │            │
   │12. FIRM ✓  │              │           │           │            │
   │<───────────│ (~6s)        │           │           │            │
```

**Your code:** Steps 2-4 (consensus→execution glue), 6-8 (batching→submission), 10-11 (finality tracking).

---

## Component Responsibilities

### `src/consensus/mod.rs`

**What it does:**
- Initialize Commonware with validator config
- Receive blocks from consensus
- Forward to execution

**What it doesn't do:**
- BFT logic (Commonware)
- P2P networking (Commonware)
- Cryptographic signing (Commonware)

---

### `src/execution/mod.rs`

**What it does:**
- Build `ExecutionPayload` from blocks
- Call `engine_newPayloadV3`
- Call `engine_forkchoiceUpdatedV3`
- Track HEAD/SAFE/FINALIZED

**What it doesn't do:**
- EVM execution (Reth)
- State storage (Reth)
- JSON-RPC serving (Reth)

---

### `src/celestia/mod.rs`

**What it does:**
- Batch blocks into blobs
- Submit via Lumina
- Track pending submissions
- Match Celestia headers to finality

**What it doesn't do:**
- Celestia protocol (Lumina)
- Data availability sampling (Lumina)
- Light client sync (Lumina)

---

### `src/types/mod.rs`

**What it does:**
- Define `Block`, `Transaction`, `Config`
- Serialization for blobs
- Type conversions

**What it doesn't do:**
- Business logic

---

### `src/main.rs`

**What it does:**
- Parse CLI args
- Load config
- Wire components together
- Run main loop

**What it doesn't do:**
- Any actual blockchain logic

---

## Where to Look When Things Break

### Symptoms → Likely Cause

| Symptom | First Check | Likely Cause |
|---------|-------------|--------------|
| Blocks not producing | Commonware logs | Consensus issue |
| Execution errors | Reth logs | Invalid payload or state |
| Blobs not landing | Celestia explorer | Lumina connection or gas |
| Finality stuck | FinalityTracker state | Celestia sync lag |
| State mismatch | Reth vs Celestia roots | Bug in your code |

### Debug Sequence

```
1. Is Commonware producing blocks?
   └─ No  → Check validator config, p2p connectivity
   └─ Yes → Continue

2. Is Reth accepting payloads?
   └─ No  → Check Engine API response, payload format
   └─ Yes → Continue

3. Are blobs landing on Celestia?
   └─ No  → Check Lumina connection, namespace, gas
   └─ Yes → Continue

4. Is finality updating?
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
│  • BFT consensus                 • Wiring components            │
│  • P2P networking                • Config management            │
│  • EVM execution                 • Batch timing                 │
│  • State tries                   • Finality state machine       │
│  • Celestia protocol             • Error handling               │
│  • DAS                           • Logging/metrics              │
│  • ZK circuits (future)          • Blob format                  │
│                                                                  │
│  ~300,000 lines                  ~1,400 lines                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

When something breaks, first ask: "Is this my 1,400 lines or their 300,000?"

Usually it's a configuration issue, not a code issue.

---

## Summary

- **You maintain plumbing**, not a blockchain
- **~1,400 lines** of glue code
- **Sequence diagrams** show exactly where your code sits
- **When debugging**, check the boundaries first
- **Most complexity** lives in dependencies you don't touch

The goal is to keep this codebase boring. Boring is good. Boring means it works.
