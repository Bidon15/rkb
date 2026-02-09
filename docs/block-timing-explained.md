# Why Block Times Are Limited to 1 Second (And How to Go Faster)

## TL;DR

| Approach | Block Time | Reth Required | Code Change |
|----------|------------|---------------|-------------|
| **Vanilla** | ~1 second | Unmodified | None |
| **Forked** | ~100-200ms | ~5 lines changed | Minimal |

Ethereum's timestamp constraint forces 1-second minimum blocks with vanilla Reth. A tiny Reth modification unlocks sub-second blocks.

---

## The Constraint

Ethereum's Yellow Paper (Section 4.3.4) specifies:

```
block.timestamp > parent.timestamp
```

Two key details:
1. **Strict inequality** - timestamps must increase, not stay equal
2. **Seconds** - timestamps are Unix seconds, not milliseconds

This was designed for Ethereum's 12-15 second blocks. It becomes a problem when you want faster blocks.

---

## The Math Problem

With 200ms block times (5 blocks per second):

```
Block 1:  timestamp = 1000  (second 1000 starts)
Block 2:  timestamp = ?     (200ms later, still second 1000)
Block 3:  timestamp = ?     (400ms later, still second 1000)
Block 4:  timestamp = ?     (600ms later, still second 1000)
Block 5:  timestamp = ?     (800ms later, still second 1000)
Block 6:  timestamp = 1001  (second 1001 starts)
```

Blocks 2-5 cannot have `timestamp > 1000` because the wall clock is still at second 1000.

**You can't have 5 blocks in 1 second when each block must have a unique, increasing second-granularity timestamp.**

---

## The Wrong Solutions

### ❌ Lie About Time

```rust
// The "dirty hack" - DON'T DO THIS
let timestamp = now.max(parent_timestamp + 1);
```

This forces timestamps forward even when the clock hasn't advanced:

| Real Time | Wall Clock | Forced Timestamp | Drift |
|-----------|------------|------------------|-------|
| T+0ms | 1000 | 1000 | 0s |
| T+200ms | 1000 | 1001 | +1s |
| T+400ms | 1000 | 1002 | +2s |
| T+600ms | 1000 | 1003 | +3s |
| T+800ms | 1000 | 1004 | +4s |

**Result:** 4 seconds of drift per real second. After 1 hour, block timestamps are ~4 hours ahead of reality.

### ❌ Use Milliseconds

```rust
// DON'T DO THIS EITHER
let timestamp = now_millis();  // 1706000000000
```

This breaks everything:

```solidity
// Solidity time constants are defined in seconds
1 days = 86400

// A time-lock contract:
require(block.timestamp > unlockTime + 1 days);

// With millisecond timestamps:
// unlockTime + 1 days = unlockTime + 86400
// But 86400 milliseconds = 86.4 seconds, not 1 day!
```

Also:
- Block explorers would show year 56,000
- Every DeFi protocol with time logic breaks
- Every indexer and tool assumes seconds

---

## The Right Solutions

### Option 1: Accept 1-Second Blocks (Vanilla Reth)

**How it works:**
- Only build a block when `wall_clock > parent.timestamp`
- Skip Simplex rounds that fall within the same second
- ~4 out of 5 rounds produce null proposals (at 200ms rounds)

```
T+0ms:    wall_clock=1000, parent=999  → 1000 > 999 ✓ → BUILD BLOCK
T+200ms:  wall_clock=1000, parent=1000 → 1000 > 1000 ✗ → SKIP
T+400ms:  wall_clock=1000, parent=1000 → 1000 > 1000 ✗ → SKIP
T+600ms:  wall_clock=1000, parent=1000 → 1000 > 1000 ✗ → SKIP
T+800ms:  wall_clock=1000, parent=1000 → 1000 > 1000 ✗ → SKIP
T+1000ms: wall_clock=1001, parent=1000 → 1001 > 1000 ✓ → BUILD BLOCK
```

**Pros:**
- Zero timestamp drift
- Works with unmodified Reth
- No maintenance burden
- Full Ethereum compatibility

**Cons:**
- 1 second confirmation latency
- Cannot achieve sub-second user experience

**Best for:** Production deployments that prioritize stability over speed.

---

### Option 2: Relax Timestamp Validation (Forked Reth)

**The change:**

```rust
// Reth: crates/consensus/common/src/validation.rs (approximate)

// BEFORE (Ethereum spec - strict inequality):
if header.timestamp <= parent.timestamp {
    return Err(ConsensusError::TimestampIsInPast);
}

// AFTER (L2/rollup mode - allow equal):
if header.timestamp < parent.timestamp {
    return Err(ConsensusError::TimestampIsInPast);
}
```

**That's it.** Change `<=` to `<`. About 5 lines of code.

**How it works:**
- Multiple blocks can share the same second-granularity timestamp
- Timestamps still accurate (they all happened during that second)
- Block ordering comes from block number, not timestamp

```
T+0ms:    wall_clock=1000 >= parent=999  ✓ → BUILD BLOCK (timestamp=1000)
T+200ms:  wall_clock=1000 >= parent=1000 ✓ → BUILD BLOCK (timestamp=1000)
T+400ms:  wall_clock=1000 >= parent=1000 ✓ → BUILD BLOCK (timestamp=1000)
T+600ms:  wall_clock=1000 >= parent=1000 ✓ → BUILD BLOCK (timestamp=1000)
T+800ms:  wall_clock=1000 >= parent=1000 ✓ → BUILD BLOCK (timestamp=1000)
T+1000ms: wall_clock=1001 >= parent=1000 ✓ → BUILD BLOCK (timestamp=1001)
```

**Pros:**
- Sub-second confirmation latency (~200ms)
- Zero timestamp drift
- Full EVM compatibility (contracts work correctly)
- Minimal code change

**Cons:**
- Must maintain a Reth fork
- Must track upstream Reth updates

**Best for:** Applications requiring fast user feedback (trading, gaming, etc.)

---

## Who Uses Forked Execution Clients?

This is not a novel approach. Major L2s do the same thing:

| Project | Client | Timestamp Change |
|---------|--------|------------------|
| **Optimism** | op-geth | Allows `>=` for L2 blocks |
| **Arbitrum** | nitro | Custom timestamp handling |
| **Rollkit** | ev-reth | Allows `>=` for sovereign rollups |
| **RKB** | ethrex | Allows `>=` for subsecond blocks |

RKB's approach is identical to what these production systems use.

### ethrex Fork Details

RKB uses a forked [ethrex](https://github.com/lambdaclass/ethrex) client with two small changes:

1. **Block validation** (`crates/common/types/block.rs`):
   ```rust
   // Change <= to <
   if header.timestamp < parent_header.timestamp {
       return Err(InvalidBlockHeaderError::TimestampLessThanParent);
   }
   ```

2. **Engine API validation** (`crates/networking/rpc/engine/fork_choice.rs`):
   ```rust
   // Change <= to <
   if attributes.timestamp < head_block.timestamp {
       return Err(RpcErr::InvalidPayloadAttributes("invalid timestamp".to_string()));
   }
   ```

Total: ~10 lines changed.

---

## Configuration

RKB provides separate Docker setups for each approach:

```
docker/
├── reth/      # Vanilla Reth (1s blocks)
│   └── config/validator-*/config.toml  # block_timing = "vanilla"
│
└── ethrex/    # Forked ethrex (subsecond blocks)
    └── config/validator-*/config.toml  # block_timing = "subsecond"
```

### Config Option

```toml
[consensus]
# "vanilla" - 1s blocks, works with unmodified Reth
# "subsecond" - ~33ms blocks, requires forked ethrex
block_timing = "vanilla"  # or "subsecond"
```

### Switching Modes

```bash
# For vanilla Reth (1s blocks):
cd docker/reth && docker compose up -d

# For forked ethrex (subsecond blocks):
cd docker/ethrex && docker compose up -d
```

---

## Choosing Your Approach

### Use Vanilla (1s blocks) if:
- You're just getting started
- You want zero maintenance burden
- 1 second latency is acceptable
- You prioritize stability over speed

### Use Forked (sub-second) if:
- You need <1s confirmation times
- You're comfortable maintaining a fork
- Your use case demands fast feedback
- You have resources to track upstream

---

## The Fork Is Tiny

To emphasize: the Reth modification is approximately **5 lines of code**.

```diff
// crates/consensus/common/src/validation.rs

- if header.timestamp <= parent.timestamp {
+ if header.timestamp < parent.timestamp {
      return Err(ConsensusError::TimestampIsInPast {
          parent_timestamp: parent.timestamp,
          timestamp: header.timestamp,
      });
  }
```

You're not rewriting the execution client. You're changing one comparison operator.

---

## Summary

```
┌─────────────────────────────────────────────────────────────────┐
│                    ETHEREUM TIMESTAMP CONSTRAINT                │
│                                                                 │
│              block.timestamp > parent.timestamp                 │
│              (seconds, strict inequality)                       │
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  VANILLA RETH (docker/reth/)     FORKED ETHREX (docker/ethrex/) │
│  ───────────────────────────     ────────────────────────────── │
│                                                                 │
│  Keep: timestamp > parent        Change to: timestamp >= parent │
│  Result: 1 block/second          Result: ~30 blocks/second      │
│  Drift: Zero                     Drift: Zero                    │
│  Maintenance: None               Maintenance: Track upstream    │
│                                                                 │
│  Best for:                       Best for:                      │
│  - Stability                     - Speed (~33ms blocks)         │
│  - Simplicity                    - Low latency UX               │
│  - Getting started               - Production L2s               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

Both approaches give you **zero timestamp drift** and **full EVM compatibility**. The only question is: do you need sub-second blocks enough to maintain a tiny fork?
