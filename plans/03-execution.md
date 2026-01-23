# Task 03: Execution Module

## Owner
Agent Session 3

## Goal
Implement Engine API client that drives vanilla reth for block execution.

## Deliverable
`src/execution/` module that:
- Connects to reth via Engine API (JSON-RPC)
- Executes blocks via `newPayload`
- Updates forkchoice for soft/firm commitments

---

## Dependencies

```toml
jsonrpsee = { version = "0.24", features = ["http-client"] }
alloy-rpc-types-engine = "0.3"
alloy-primitives = "0.8"
alloy-rlp = "0.3"
```

---

## Interface

```rust
// src/execution/mod.rs

pub struct ExecutionConfig {
    pub reth_url: String,      // e.g., "http://localhost:8551"
    pub jwt_secret: [u8; 32],  // For Engine API auth
}

pub struct ExecutionClient {
    // JSON-RPC client
}

impl ExecutionClient {
    pub async fn new(config: ExecutionConfig) -> Result<Self>;

    /// Execute block, returns execution result
    pub async fn execute_block(&self, block: &Block) -> Result<ExecutionResult>;

    /// Update soft commitment (HEAD + SAFE)
    pub async fn update_soft(&self, block_hash: B256) -> Result<()>;

    /// Update firm commitment (FINALIZED)
    pub async fn update_firm(&self, block_hash: B256) -> Result<()>;

    /// Get current forkchoice state
    pub async fn get_forkchoice(&self) -> Result<ForkchoiceState>;
}

pub struct ExecutionResult {
    pub block_hash: B256,
    pub block_number: u64,
    pub gas_used: u64,
    pub status: ExecutionStatus,
}

pub enum ExecutionStatus {
    Valid,
    Invalid { reason: String },
    Syncing,
}

pub struct ForkchoiceState {
    pub head: B256,
    pub safe: B256,
    pub finalized: B256,
}
```

---

## Tasks

### 1. Project Setup
- [ ] Create `src/execution/mod.rs`
- [ ] Add dependencies (jsonrpsee, alloy)
- [ ] Define types

### 2. Engine API Client
- [ ] JWT authentication setup
- [ ] HTTP client to reth
- [ ] Basic connectivity test

### 3. Block Execution
- [ ] Build `ExecutionPayload` from Block
- [ ] Encode transactions (RLP)
- [ ] Call `engine_newPayloadV3`
- [ ] Parse response

### 4. Forkchoice Updates
- [ ] `engine_forkchoiceUpdatedV3` for soft
- [ ] `engine_forkchoiceUpdatedV3` for firm
- [ ] Handle response status

### 5. State Machine
- [ ] Track current soft/firm heads
- [ ] Ensure soft >= firm invariant
- [ ] Handle reorg scenarios (if any)

### 6. Testing
- [ ] Mock reth for unit tests
- [ ] Integration test with actual reth

---

## Engine API Calls

### newPayload

```rust
async fn execute_block(&self, block: &Block) -> Result<ExecutionResult> {
    let payload = ExecutionPayloadV3 {
        parent_hash: block.parent_hash.into(),
        fee_recipient: Address::ZERO,  // Or configured
        state_root: B256::ZERO,        // Reth computes this
        receipts_root: B256::ZERO,
        logs_bloom: Bloom::ZERO,
        prev_randao: B256::ZERO,
        block_number: block.height,
        gas_limit: 30_000_000,
        gas_used: 0,                   // Reth computes this
        timestamp: block.timestamp,
        extra_data: Bytes::new(),
        base_fee_per_gas: U256::from(1_000_000_000),  // 1 gwei
        block_hash: B256::ZERO,        // Reth computes this
        transactions: encode_transactions(&block.transactions),
        withdrawals: vec![],
        blob_gas_used: 0,
        excess_blob_gas: 0,
    };

    let result = self.client
        .request("engine_newPayloadV3", (payload, vec![], parent_beacon_root))
        .await?;

    // Parse PayloadStatusV1
}
```

### forkchoiceUpdated

```rust
async fn update_forkchoice(
    &self,
    head: B256,
    safe: B256,
    finalized: B256,
) -> Result<()> {
    let state = ForkchoiceStateV1 {
        head_block_hash: head,
        safe_block_hash: safe,
        finalized_block_hash: finalized,
    };

    let result = self.client
        .request("engine_forkchoiceUpdatedV3", (state, None::<PayloadAttributes>))
        .await?;

    // Parse ForkchoiceUpdatedResponse
}
```

---

## Soft/Firm Mapping

```
Soft confirmation (from PoA):
  → head_block_hash = block.hash
  → safe_block_hash = block.hash
  → finalized_block_hash = (unchanged)

Firm confirmation (from Celestia):
  → head_block_hash = (unchanged)
  → safe_block_hash = (unchanged)
  → finalized_block_hash = block.hash
```

---

## Research Needed

1. **Engine API V3 Spec**
   - Exact field requirements
   - Cancun/Deneb specific fields

2. **JWT Auth**
   - How reth expects JWT
   - Token format

3. **Transaction Encoding**
   - RLP encoding for ExecutionPayload.transactions
   - Alloy types for this

---

## Out of Scope

- Block building (consensus does this)
- Transaction pool (consensus manages)
- Direct state access (use standard JSON-RPC)

---

## Test Criteria

```bash
# Payload construction
cargo test execution::build_payload

# JWT auth
cargo test execution::jwt_authentication

# Full execution with reth
cargo test execution::execute_with_reth --ignored
```

---

## Output

When complete, main loop can:

```rust
use crate::execution::{ExecutionClient, ExecutionConfig};

let execution = ExecutionClient::new(config).await?;

// On new block from consensus
let result = execution.execute_block(&block).await?;
execution.update_soft(result.block_hash).await?;

// On finality from Celestia
execution.update_firm(block_hash).await?;
```
