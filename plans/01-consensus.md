# Task 01: Consensus Module

## Owner
Agent Session 1

## Goal
Implement PoA consensus using commonware-consensus that produces ordered blocks.

## Deliverable
`src/consensus/` module that:
- Manages a static validator set
- Produces blocks with ordered transactions
- Exposes block stream for downstream consumers

---

## Dependencies

```toml
commonware-consensus = "0.0.65"
commonware-p2p = "0.0.65"
commonware-cryptography = "0.0.65"
commonware-runtime = "0.0.65"
```

---

## Interface

```rust
// src/consensus/mod.rs

pub struct ConsensusConfig {
    pub validators: Vec<PublicKey>,
    pub private_key: PrivateKey,
    pub listen_addr: SocketAddr,
    pub peers: Vec<SocketAddr>,
}

pub struct Consensus {
    // ...
}

impl Consensus {
    pub async fn new(config: ConsensusConfig) -> Result<Self>;

    /// Returns stream of finalized blocks
    pub fn block_stream(&self) -> impl Stream<Item = Block>;

    /// Submit transaction to mempool
    pub async fn submit_tx(&self, tx: Transaction) -> Result<()>;
}

pub struct Block {
    pub height: u64,
    pub timestamp: u64,
    pub parent_hash: [u8; 32],
    pub transactions: Vec<Transaction>,
    pub proposer: PublicKey,
    pub signatures: Vec<Signature>,  // Validator signatures
}

pub struct Transaction {
    pub data: Vec<u8>,  // Opaque EVM transaction bytes
}
```

---

## Tasks

### 1. Project Setup
- [ ] Create `src/consensus/mod.rs`
- [ ] Add commonware dependencies to Cargo.toml
- [ ] Define basic types (Block, Transaction)

### 2. Consensus Engine
- [ ] Initialize commonware-consensus Simplex engine
- [ ] Configure for PoA (fixed validator set)
- [ ] Implement leader rotation

### 3. Networking
- [ ] Setup commonware-p2p
- [ ] Peer discovery (static list for now)
- [ ] Transaction gossip
- [ ] Block propagation

### 4. Block Production
- [ ] Mempool for pending transactions
- [ ] Block building (collect txs, create block)
- [ ] Block signing
- [ ] Emit finalized blocks on stream

### 5. Testing
- [ ] Unit tests for block production
- [ ] Integration test with 3 validators
- [ ] Test leader rotation

---

## Research Needed

1. **commonware-consensus Simplex API**
   - How to configure validator set
   - How to hook into block production
   - Message types and handlers

2. **Signature Scheme**
   - Ed25519 (commonware native) vs secp256k1 (Ethereum)
   - Decision affects key management

---

## Out of Scope

- Dynamic validator set (future work)
- Slashing (future work)
- MEV/PBS (future work)

---

## Test Criteria

```bash
# Single node produces blocks
cargo test consensus::single_node_produces_blocks

# Three nodes reach consensus
cargo test consensus::three_node_consensus

# Transactions get included
cargo test consensus::tx_inclusion
```

---

## Output

When complete, other modules can:

```rust
use crate::consensus::{Consensus, ConsensusConfig};

let consensus = Consensus::new(config).await?;

// Submit transactions
consensus.submit_tx(tx).await?;

// Consume finalized blocks
let mut blocks = consensus.block_stream();
while let Some(block) = blocks.next().await {
    // Send to Celestia (02-celestia)
    // Send to Execution (03-execution)
}
```
