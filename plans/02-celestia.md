# Task 02: Celestia Module

## Owner
Agent Session 2

## Goal
Implement Celestia DA integration using Lumina TX client for blob submission and finality tracking.

## Deliverable
`src/celestia/` module that:
- Submits blocks as blobs to Celestia
- Tracks Celestia finality (when blobs are included in finalized Celestia blocks)
- Emits firm confirmation events to trigger reth FINALIZED updates

---

## Dependencies

```toml
lumina-rpc = "..."       # Lumina RPC client
celestia-types = "..."   # Celestia types (Namespace, Blob, etc.)
```

---

## Configuration

```rust
pub struct CelestiaConfig {
    /// Celestia RPC endpoint for submitting blobs
    pub submit_endpoint: String,  // e.g., "https://celestia-rpc.example.com:26658"

    /// Celestia RPC endpoint for querying (can be same as submit)
    pub query_endpoint: String,

    /// Account key for signing blob submissions
    pub account_key_path: PathBuf,

    /// Namespace for this chain's blobs
    pub namespace: Namespace,

    /// Gas price for submissions (in utia)
    pub gas_price: f64,
}
```

```toml
# config.toml
[celestia]
submit_endpoint = "https://celestia-consensus.example.com:26658"
query_endpoint = "https://celestia-data.example.com:26658"
account_key_path = "/config/celestia-key"
namespace = "my_chain_namespace"
gas_price = 0.002
```

---

## Lumina Client API (from eigerco/lumina)

### BlobApi
```rust
// Submit blobs
blob().submit(&[blob], tx_config) -> AsyncGrpcCall<TxInfo>

// Get blob by commitment
blob().get(height, namespace, commitment) -> Result<Blob>

// Get all blobs at height
blob().get_all(height, namespaces) -> Result<Option<Vec<Blob>>>

// Check if blob is included
blob().included(height, namespace, proof, commitment) -> Result<bool>

// Subscribe to new blobs in namespace
blob().subscribe(namespace) -> Stream<BlobsAtHeight>
```

### HeaderApi
```rust
// Get latest synced header
header().head() -> Result<ExtendedHeader>

// Get network head (most recent announced)
header().network_head() -> Result<ExtendedHeader>

// Wait until height is synced
header().wait_for_height(height) -> Result<ExtendedHeader>

// Subscribe to new headers
header().subscribe() -> Stream<ExtendedHeader>
```

---

## Interface

```rust
// src/celestia/mod.rs

pub struct CelestiaClient {
    client: lumina_rpc::Client,
    config: CelestiaConfig,
    finality_tracker: FinalityTracker,
}

impl CelestiaClient {
    pub async fn new(config: CelestiaConfig) -> Result<Self>;

    /// Submit block as blob, returns submission info
    pub async fn submit_block(&self, block: &Block) -> Result<CelestiaSubmission>;

    /// Start finality tracking loop, returns stream of confirmations
    pub fn track_finality(&self) -> impl Stream<Item = FinalityConfirmation>;

    /// Get block from Celestia (for sync/recovery)
    pub async fn get_block(&self, height: u64) -> Result<Option<Block>>;

    /// Verify a block exists at Celestia height
    pub async fn verify_inclusion(&self, height: u64, commitment: &[u8; 32]) -> Result<bool>;
}

pub struct CelestiaSubmission {
    pub block_hash: B256,          // Our block hash
    pub celestia_height: u64,      // Celestia height where blob landed
    pub commitment: Commitment,     // Blob commitment for verification
    pub namespace: Namespace,
}

pub struct FinalityConfirmation {
    pub block_hash: B256,          // Our block that is now final
    pub celestia_height: u64,      // Celestia height that finalized
    pub celestia_header_hash: Hash, // Celestia block hash (for proofs)
}
```

---

## Finality Tracking (Critical)

### How Celestia Finality Works

```
┌─────────────────────────────────────────────────────────────────┐
│ CELESTIA CONSENSUS                                              │
│                                                                 │
│ 1. We submit blob → included in Celestia block at height H      │
│ 2. Celestia block H gets 2/3+ validator signatures              │
│ 3. Block H is "committed" (soft finality)                       │
│ 4. After ~12s, block H is considered final                      │
│                                                                 │
│ Finality = When header().wait_for_height(H) returns             │
│            (means our light client has verified the header)     │
└─────────────────────────────────────────────────────────────────┘
```

### Finality Tracker State Machine

```rust
// src/celestia/finality.rs

pub struct FinalityTracker {
    /// Blocks awaiting Celestia finality
    pending: HashMap<B256, PendingFinality>,

    /// Channel to send finality confirmations
    confirmations_tx: mpsc::Sender<FinalityConfirmation>,
}

pub struct PendingFinality {
    pub block_hash: B256,           // Our block hash
    pub block_height: u64,          // Our block height
    pub celestia_height: u64,       // Celestia height where blob is
    pub commitment: Commitment,      // For verification
    pub submitted_at: Instant,
}

impl FinalityTracker {
    /// Track a new submission
    pub fn track(&mut self, block_hash: B256, submission: CelestiaSubmission) {
        self.pending.insert(block_hash, PendingFinality {
            block_hash,
            block_height: submission.block_height,
            celestia_height: submission.celestia_height,
            commitment: submission.commitment,
            submitted_at: Instant::now(),
        });
    }

    /// Called when Celestia header is verified at height
    pub async fn on_celestia_header(&mut self, header: &ExtendedHeader) {
        let finalized_height = header.height().value();

        // Find all pending blocks at or below this height
        let newly_finalized: Vec<_> = self.pending
            .iter()
            .filter(|(_, p)| p.celestia_height <= finalized_height)
            .map(|(hash, p)| (*hash, p.clone()))
            .collect();

        // Emit confirmations in block order
        let mut sorted: Vec<_> = newly_finalized.into_iter().collect();
        sorted.sort_by_key(|(_, p)| p.block_height);

        for (hash, pending) in sorted {
            let confirmation = FinalityConfirmation {
                block_hash: hash,
                celestia_height: pending.celestia_height,
                celestia_header_hash: header.hash(),
            };

            self.confirmations_tx.send(confirmation).await.ok();
            self.pending.remove(&hash);
        }
    }
}
```

### Finality Loop

```rust
// src/celestia/mod.rs

impl CelestiaClient {
    /// Main finality tracking loop
    pub async fn run_finality_tracker(&self) -> Result<()> {
        // Subscribe to new Celestia headers
        let mut headers = self.client.header().subscribe();

        while let Some(header_result) = headers.next().await {
            match header_result {
                Ok(header) => {
                    self.finality_tracker.on_celestia_header(&header).await;
                }
                Err(e) => {
                    warn!("Header subscription error: {}", e);
                    // Reconnect logic here
                }
            }
        }

        Ok(())
    }

    /// Alternative: Poll-based finality check
    pub async fn check_finality(&self, submission: &CelestiaSubmission) -> Result<bool> {
        // Wait for Celestia to sync to submission height
        let header = self.client
            .header()
            .wait_for_height(submission.celestia_height)
            .await?;

        // Verify blob is actually included
        let included = self.client
            .blob()
            .included(
                submission.celestia_height,
                submission.namespace.clone(),
                &submission.proof,
                submission.commitment.clone(),
            )
            .await?;

        Ok(included)
    }
}
```

---

## Complete Flow: Consensus → Celestia → Reth

```
┌─────────────────────────────────────────────────────────────────┐
│ SEQUENCER MAIN LOOP                                             │
└─────────────────────────────────────────────────────────────────┘

// Spawn finality tracker
let (finality_tx, mut finality_rx) = mpsc::channel(100);
let celestia = CelestiaClient::new(config, finality_tx).await?;
tokio::spawn(celestia.run_finality_tracker());

// Main loop
loop {
    tokio::select! {
        // New block from consensus
        Some(block) = consensus.block_stream().next() => {
            // 1. Execute immediately (soft)
            let result = execution.execute_block(&block).await?;
            execution.update_soft(result.block_hash).await?;

            // 2. Submit to Celestia (non-blocking)
            let submission = celestia.submit_block(&block).await?;

            // 3. Track for finality
            celestia.track_finality(block.hash, submission);

            info!(
                block = block.height,
                celestia_height = submission.celestia_height,
                "Block executed (soft), submitted to Celestia"
            );
        }

        // Finality confirmation from Celestia
        Some(confirmation) = finality_rx.recv() => {
            // 4. Update reth FINALIZED
            execution.update_firm(confirmation.block_hash).await?;

            info!(
                block_hash = %confirmation.block_hash,
                celestia_height = confirmation.celestia_height,
                "Block finalized (firm) via Celestia"
            );
        }
    }
}
```

---

## Tasks

### 1. Project Setup
- [ ] Create `src/celestia/mod.rs`
- [ ] Create `src/celestia/finality.rs`
- [ ] Add Lumina dependencies to Cargo.toml
- [ ] Define types (CelestiaSubmission, FinalityConfirmation, PendingFinality)

### 2. Lumina Client Setup
- [ ] Initialize Lumina RPC client with endpoints
- [ ] Load account key for signing
- [ ] Configure TxConfig with gas price

### 3. Blob Submission
- [ ] Encode Block → Blob format (bincode or protobuf)
- [ ] Call `blob().submit()` with TxConfig
- [ ] Parse TxInfo response for height and commitment
- [ ] Handle submission errors/retries

### 4. Finality Tracking
- [ ] Implement FinalityTracker state machine
- [ ] Subscribe to Celestia headers via `header().subscribe()`
- [ ] Match finalized heights to pending submissions
- [ ] Emit FinalityConfirmation events in block order

### 5. Block Retrieval (for sync)
- [ ] Fetch blobs via `blob().get_all(height, namespaces)`
- [ ] Decode Blob → Block
- [ ] Verify blob integrity via commitment

### 6. Testing
- [ ] Unit tests with mock client
- [ ] Integration test with Celestia Mocha testnet

---

## Blob Encoding

```rust
// Simple encoding: length-prefixed protobuf or bincode
fn encode_block(block: &Block) -> Blob {
    let data = bincode::serialize(block).unwrap();
    Blob::new(namespace, data)
}

fn decode_block(blob: &Blob) -> Result<Block> {
    bincode::deserialize(&blob.data)
}
```

---

## Namespace Strategy

Each chain deployment gets its own Celestia namespace:
```rust
// Derived from chain_id or configured explicitly
let namespace = Namespace::new_v0(&chain_id.to_be_bytes())?;
```

---

## Research Needed

1. **Lumina API**
   - Blob submission methods
   - Finality notifications
   - Light client sync behavior

2. **Celestia Finality**
   - How long until blob is final?
   - What events does Lumina expose?

---

## Out of Scope

- Running full Celestia node (we use light client)
- Multi-namespace (one namespace per chain)
- Blob compression (future optimization)

---

## Test Criteria

```bash
# Blob encoding roundtrip
cargo test celestia::blob_encoding_roundtrip

# Submit to testnet
cargo test celestia::submit_to_mocha --ignored

# Finality tracking
cargo test celestia::finality_confirmation
```

---

## Output

When complete, main loop can:

```rust
use crate::celestia::{CelestiaClient, CelestiaConfig};

let celestia = CelestiaClient::new(config).await?;

// On new block from consensus
let submission = celestia.submit_block(&block).await?;
println!("Submitted at Celestia height {}", submission.celestia_height);

// Track finality
let mut finality = celestia.finality_stream();
while let Some(confirmation) = finality.next().await {
    // Update firm commitment in execution (03-execution)
}
```
