# ZK Verification Roadmap

A future development path for adding trust-minimized verification to the PoA EVM stack using RSP (Reth Succinct Processor). This document serves as a reference point if this repo gains traction or when time and resources allow.

---

## Table of Contents

- [ZK Verification Roadmap](#zk-verification-roadmap)
  - [Table of Contents](#table-of-contents)
  - [Why ZK](#why-zk)
  - [System Overview](#system-overview)
  - [Finality Tiers](#finality-tiers)
  - [Components](#components)
    - [RSP Prover](#rsp-prover)
    - [Proof Queue](#proof-queue)
    - [Celestia Blob Structure](#celestia-blob-structure)
    - [Verification SDK](#verification-sdk)
  - [Data Flow](#data-flow)
    - [Block Production (Real-time)](#block-production-real-time)
    - [Proof Generation (Async)](#proof-generation-async)
    - [Verification (On-demand)](#verification-on-demand)
  - [Commitment-Linked Chains](#commitment-linked-chains)
  - [Verification Infrastructure](#verification-infrastructure)
    - [On-Chain (Bridges)](#on-chain-bridges)
    - [Off-Chain (Apps, Solvers)](#off-chain-apps-solvers)
  - [Implementation Phases](#implementation-phases)
    - [Phase 1: Proof Pipeline](#phase-1-proof-pipeline)
    - [Phase 2: Verification SDK](#phase-2-verification-sdk)
    - [Phase 3: On-Chain Verification](#phase-3-on-chain-verification)
    - [Phase 4: State Proof Service](#phase-4-state-proof-service)
    - [Phase 5: Proof Aggregation (Future)](#phase-5-proof-aggregation-future)
  - [Open Questions](#open-questions)
    - [Proving Economics](#proving-economics)
    - [Proof Liveness](#proof-liveness)
    - [Blob Format](#blob-format)
  - [Summary](#summary)
  - [Prerequisites](#prerequisites)

---

## Why ZK

The chain works without ZK. Validators sync, users transact, bridges move funds. So why consider adding it?

**The answer isn't "ZK is better." The answer is: different users need different trust guarantees.**

| User                    | What They Need           | Current Solution            |
| ----------------------- | ------------------------ | --------------------------- |
| Regular users           | Fast confirmations       | PoA (200ms) works fine      |
| Fast bridges            | Quick finality           | Celestia DA (6s) works fine |
| Trust-minimized bridges | Cryptographic proof      | **Would need ZK**           |
| Intent solvers          | Verified settlement      | **Would need ZK**           |
| Light clients           | Verify without full node | **Would need ZK**           |

ZK proofs would let anyone verify execution correctness without trusting the PoA validators. This unlocks trust-minimized bridging and settlement.

---

## System Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         SEQUENCER                                │
│                                                                  │
│   Commonware ──→ Reth ──→ Celestia Submitter                    │
│   (ordering)    (exec)   (data blobs)                           │
│                   │                                              │
│                   ▼                                              │
│              RSP Prover ──→ Celestia Submitter                  │
│              (async)       (proof blobs)                        │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
                        ┌───────────┐
                        │  Celestia │
                        └───────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │    Verifiers    │
                    │  (bridges, apps,│
                    │   solvers)      │
                    └─────────────────┘
```

**Key design principle:** Proof generation would be fully decoupled from block production. The chain runs at full speed regardless of proving latency.

---

## Finality Tiers

Three tiers would serve different trust/latency trade-offs:

| Tier         | Source             | Latency   | Trust Model                 | Use Case                           |
| ------------ | ------------------ | --------- | --------------------------- | ---------------------------------- |
| **Soft**     | PoA consensus      | ~200ms    | Trust validators            | UX, gaming, low value              |
| **DA Final** | Celestia inclusion | ~6s       | Trust validators + Celestia | Fast bridges, normal ops           |
| **Proven**   | ZK proof posted    | ~1~30min+ | Trust math only             | Trustless bridges, large transfers |

**The insight:** Most users would never need the proven tier. It exists for high-value, trust-minimized use cases.

---

## Components

### RSP Prover

[Succinct's RSP](https://github.com/succinctlabs/rsp) re-executes blocks in SP1's zkVM to generate validity proofs.

**Integration model:**

- RSP connects to reth via RPC
- Fetches block data, re-executes in zkVM
- Outputs ZK proof attesting to correct execution
- No modifications to RSP needed - use as external tool

**Why RSP:**

- Proves vanilla reth execution
- No custom circuits needed
- Maintained by Succinct
- SP1 proofs verify on any EVM chain

### Proof Queue

Would manage the async relationship between block production and proving.

**States:**

- **Pending** - Batch posted to Celestia, awaiting proof
- **In Progress** - RSP actively proving
- **Completed** - Proof ready for Celestia posting

**Reality check:** At 2 Ggas/s with 100-200ms blocks, proving would lag by 30+ minutes. This is by design - the chain doesn't wait for proofs.

### Celestia Blob Structure

Two blob types in the same namespace:

**Data Blobs** (posted every ~5 seconds)

- Batch sequence number
- Blocks with transactions
- Pre/post state roots
- PoA signatures
- Previous data blob commitment (backward link)

**Proof Blobs** (posted when proofs complete)

- Proof sequence number
- Batch range this proof covers
- Pre/post state roots
- Previous proof blob commitment (backward link)
- Reference to data blob range being proven
- The ZK proof

**Why separate blobs:** Proofs arrive async. Mixing them with data would either delay data posting or create complex ordering logic.

### Verification SDK

Developers shouldn't need to read Succinct's repos to verify proofs. A complete SDK would include:

| Component                 | Purpose                                      |
| ------------------------- | -------------------------------------------- |
| **Status API**            | REST/WebSocket for "is batch X proven?"      |
| **verifier-js**           | TypeScript SDK for browser/Node verification |
| **verifier-rs**           | Rust SDK for backend verification            |
| **YourChainVerifier.sol** | On-chain verification for bridges            |
| **State Proof Service**   | Merkle proofs against verified state roots   |
| **Bridge Template**       | Ready-to-deploy trustless bridge             |

**The principle:** One `npm install` to verify. No digging through zkVM internals.

---

## Data Flow

### Block Production (Real-time)

```
Transactions → Commonware orders → Reth executes → Data batch to Celestia
                                                          │
                                                    (~5 seconds)
```

### Proof Generation (Async)

```
Data batch posted → Added to proof queue → RSP proves → Proof batch to Celestia
                                                              │
                                                        (~30 minutes)
```

### Verification (On-demand)

```
Bridge receives claim → Fetches proof from Celestia → Verifies on-chain → Releases funds
```

---

## Commitment-Linked Chains

Both data and proof blobs would form backward-linked chains via Celestia commitments:

```
DATA CHAIN:
D₁ ←── D₂ ←── D₃ ←── D₄ ←── D₅
│      │      │      │      │
C_D1   C_D2   C_D3   C_D4   C_D5

PROOF CHAIN:
P₁ ←──────── P₂ ←──────── P₃
│            │            │
C_P1         C_P2         C_P3
│            │            │
proves       proves       proves
D₁-D₁₀       D₁₁-D₂₀     D₂₁-D₃₀
```

**Why commitment links:**

1. **Verifiable traversal** - Walk backwards to verify chain integrity
2. **Efficient queries** - Find proofs without external index
3. **Proof aggregation** - Future recursive SNARKs can aggregate ranges

---

## Verification Infrastructure

### On-Chain (Bridges)

```
ETHEREUM
┌─────────────────────────────────────────┐
│  SP1Verifier (Succinct's contract)      │
│           ▲                             │
│           │                             │
│  YourChainVerifier (wrapper)            │
│           ▲                             │
│           │                             │
│  Bridge Contract (developer's)          │
└─────────────────────────────────────────┘
```

SP1 proofs verify in ~300k gas. Reasonable for high-value transfers.

### Off-Chain (Apps, Solvers)

- **Status API** - "Is my transaction proven yet?"
- **SDK verification** - Verify proofs locally without trusting anyone
- **State proofs** - Prove specific balances/storage against verified roots

---

## Implementation Phases

### Phase 1: Proof Pipeline

| Task                | Description                          |
| ------------------- | ------------------------------------ |
| Proof queue service | Track batches pending proof          |
| RSP integration     | Shell out to RSP or use as library   |
| Proof blob format   | Define schema with commitment links  |
| Celestia posting    | Submit proof blobs to same namespace |

**Deliverable:** Proofs appearing on Celestia, trailing data by ~1~30min.

### Phase 2: Verification SDK

| Task          | Description                            |
| ------------- | -------------------------------------- |
| Status API    | REST endpoints for proof status        |
| verifier-js   | TypeScript SDK with local verification |
| verifier-rs   | Rust SDK for backends                  |
| Documentation | Integration guides                     |

**Deliverable:** Developers can check proof status and verify off-chain.

### Phase 3: On-Chain Verification

| Task                    | Description                      |
| ----------------------- | -------------------------------- |
| YourChainVerifier.sol   | Wrapper around SP1 verifier      |
| State inclusion helpers | Verify Merkle proofs on-chain    |
| Deploy to mainnets      | Ethereum, Arbitrum, Base, etc.   |
| Bridge template         | Ready-to-deploy trustless bridge |

**Deliverable:** Bridges can verify proofs on-chain.

### Phase 4: State Proof Service

| Task                    | Description                              |
| ----------------------- | ---------------------------------------- |
| Merkle proof generation | Prove account/storage against state root |
| Batch queries           | Multiple proofs in one request           |
| Caching layer           | Don't regenerate proofs repeatedly       |

**Deliverable:** Complete verification toolkit for any state query.

### Phase 5: Proof Aggregation (Future)

| Task                 | Description                         |
| -------------------- | ----------------------------------- |
| Recursive SNARK      | Aggregate multiple proofs into one  |
| Aggregation service  | Periodically aggregate proof ranges |
| Cheaper verification | Single proof covers hours/days      |

**Deliverable:** Reduced verification costs for bridges.

---

## Open Questions

### Proving Economics

| Question              | Options                                                |
| --------------------- | ------------------------------------------------------ |
| Who pays for proving? | Sequencer operator / Bridge operators / Fee from users |
| Proving hardware?     | Run own / Succinct's prover network / Open market      |
| Cost at scale?        | Need benchmarks at target throughput                   |

### Proof Liveness

| Question              | Options                                       |
| --------------------- | --------------------------------------------- |
| What if prover fails? | Anyone can generate proofs (deterministic)    |
| Alerting?             | Monitor proof lag, alert if exceeds threshold |
| Fallback?             | PoA signatures with timeout (degraded trust)  |

### Blob Format

| Question     | Options                              |
| ------------ | ------------------------------------ |
| Encoding?    | Protobuf / Bincode / SSZ             |
| Compression? | Worth it for proofs? (~1-2KB anyway) |
| Versioning?  | How to handle format upgrades        |

---

## Summary

ZK verification would be an opt-in layer for users who need trustless guarantees. The chain operates normally without it. Proofs lag behind execution - that's a feature, not a bug.

**What would need to be built:**

- Proof pipeline (queue → RSP → Celestia)
- Commitment-linked blob structure
- Verification SDKs (JS, Rust, Solidity)
- Status API and state proof service

**What can be used unchanged:**

- Commonware (consensus)
- Reth (execution)
- RSP (proving)
- Celestia (DA)

The hard ZK math is solved by Succinct. The work here is plumbing and DevX.

---

## Prerequisites

Before starting this roadmap:

- Core PoA + Celestia DA flow working
- Stable blob format for data batches
- Reth RPC accessible for RSP

This roadmap assumes the base chain is operational and focuses purely on the ZK verification layer.
