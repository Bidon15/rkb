# Task 05: Integration

## Owner
All Agents (After 01-04 complete)

## Goal
Wire all modules together into a single sequencer binary.

## Deliverable
- `src/main.rs` - CLI entrypoint
- `src/sequencer.rs` - Main orchestration loop
- Working end-to-end flow

---

## Dependencies

All modules from 01-04:
```rust
use crate::consensus::{Consensus, ConsensusConfig};
use crate::celestia::{CelestiaClient, CelestiaConfig};
use crate::execution::{ExecutionClient, ExecutionConfig};
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         SEQUENCER                               │
│                                                                 │
│  ┌─────────────┐                                                │
│  │   Config    │                                                │
│  └──────┬──────┘                                                │
│         │                                                       │
│         ▼                                                       │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                    Main Loop                            │    │
│  │                                                         │    │
│  │   Consensus ──► Block ──┬──► Execution (soft)          │    │
│  │      │                  │                               │    │
│  │      │                  └──► Celestia ──► Finality ──►  │    │
│  │      │                                    Execution     │    │
│  │      │                                    (firm)        │    │
│  │      ▼                                                  │    │
│  │   Mempool ◄── RPC                                       │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## Main Loop

```rust
// src/sequencer.rs

pub struct Sequencer {
    consensus: Consensus,
    celestia: CelestiaClient,
    execution: ExecutionClient,
    state: SequencerState,
    finality_rx: mpsc::Receiver<FinalityConfirmation>,
}

struct SequencerState {
    /// Latest soft-confirmed block (executed, not yet Celestia-final)
    soft_head: B256,

    /// Latest firm-confirmed block (Celestia finalized)
    firm_head: B256,

    /// Our block height → Celestia submission info
    /// Used for debugging/metrics, finality tracked in CelestiaClient
    submissions: HashMap<u64, CelestiaSubmission>,
}

impl Sequencer {
    pub async fn new(config: Config) -> Result<Self> {
        // Create finality channel
        let (finality_tx, finality_rx) = mpsc::channel(100);

        // Initialize components
        let consensus = Consensus::new(config.consensus).await?;
        let celestia = CelestiaClient::new(config.celestia, finality_tx).await?;
        let execution = ExecutionClient::new(config.execution).await?;

        // Start Celestia finality tracker in background
        let celestia_clone = celestia.clone();
        tokio::spawn(async move {
            if let Err(e) = celestia_clone.run_finality_tracker().await {
                error!("Finality tracker error: {}", e);
            }
        });

        Ok(Self {
            consensus,
            celestia,
            execution,
            state: SequencerState::default(),
            finality_rx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut blocks = self.consensus.block_stream();

        loop {
            tokio::select! {
                // New block from PoA consensus
                Some(block) = blocks.next() => {
                    self.handle_new_block(block).await?;
                }

                // Finality confirmation from Celestia (via channel)
                Some(confirmation) = self.finality_rx.recv() => {
                    self.handle_finality(confirmation).await?;
                }
            }
        }
    }

    async fn handle_new_block(&mut self, block: Block) -> Result<()> {
        let block_hash = block.hash();
        let block_height = block.height;

        // ┌─────────────────────────────────────────────────────────────┐
        // │ STEP 1: Execute on Reth (SOFT CONFIRMATION)                 │
        // │                                                             │
        // │ User sees tx as "confirmed" after this                      │
        // └─────────────────────────────────────────────────────────────┘
        let result = self.execution.execute_block(&block).await?;

        // Update forkchoice: HEAD and SAFE move forward, FINALIZED unchanged
        self.execution.forkchoice_update(
            head: result.block_hash,
            safe: result.block_hash,
            finalized: self.state.firm_head,  // Unchanged
        ).await?;

        self.state.soft_head = result.block_hash;

        info!(
            height = block_height,
            hash = %result.block_hash,
            "SOFT: Block executed on Reth"
        );

        // ┌─────────────────────────────────────────────────────────────┐
        // │ STEP 2: Submit to Celestia                                  │
        // │                                                             │
        // │ Blob goes to Celestia mempool, will be included in next     │
        // │ Celestia block                                              │
        // └─────────────────────────────────────────────────────────────┘
        let submission = self.celestia.submit_block(&block).await?;

        // Track submission (finality tracker will notify us when Celestia finalizes)
        self.celestia.track_finality(block_hash, submission.clone());
        self.state.submissions.insert(block_height, submission.clone());

        info!(
            height = block_height,
            celestia_height = submission.celestia_height,
            "PENDING: Submitted to Celestia, awaiting finality"
        );

        Ok(())
    }

    async fn handle_finality(&mut self, confirmation: FinalityConfirmation) -> Result<()> {
        // ┌─────────────────────────────────────────────────────────────┐
        // │ STEP 3: Celestia Finalized (FIRM CONFIRMATION)              │
        // │                                                             │
        // │ Celestia block containing our blob is now final.            │
        // │ Update Reth FINALIZED forkchoice.                           │
        // │                                                             │
        // │ User sees tx as "finalized" after this.                     │
        // │ Safe for bridges, withdrawals, high-value operations.       │
        // └─────────────────────────────────────────────────────────────┘

        // Update forkchoice: Only FINALIZED moves forward
        self.execution.forkchoice_update(
            head: self.state.soft_head,        // Unchanged
            safe: self.state.soft_head,        // Unchanged
            finalized: confirmation.block_hash, // Moves forward
        ).await?;

        self.state.firm_head = confirmation.block_hash;

        info!(
            block_hash = %confirmation.block_hash,
            celestia_height = confirmation.celestia_height,
            celestia_block = %confirmation.celestia_header_hash,
            "FIRM: Block finalized via Celestia"
        );

        // Emit metrics
        metrics::gauge!("sequencer.firm_head").set(confirmation.block_hash);
        metrics::histogram!("sequencer.finality_latency").record(
            confirmation.finality_latency_ms()
        );

        Ok(())
    }
}
```

---

## Configuration

```rust
// src/config.rs

#[derive(Debug, Deserialize)]
pub struct Config {
    pub chain_id: u64,
    pub consensus: ConsensusConfig,
    pub celestia: CelestiaConfig,
    pub execution: ExecutionConfig,
    pub rpc: Option<RpcConfig>,
}

#[derive(Debug, Deserialize)]
pub struct RpcConfig {
    pub listen_addr: SocketAddr,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content)
    }
}
```

```toml
# config.toml example

chain_id = 12345

[consensus]
validators = [
    "0x1234...",
    "0x5678...",
    "0x9abc...",
]
private_key_path = "/etc/sequencer/key"
listen_addr = "0.0.0.0:26656"
peers = [
    "10.0.0.2:26656",
    "10.0.0.3:26656",
]

[celestia]
network = "mainnet"  # or "mocha", "arabica"
namespace = "my_chain"

[execution]
reth_url = "http://localhost:8551"
jwt_secret_path = "/etc/sequencer/jwt.hex"

[rpc]
listen_addr = "0.0.0.0:8080"
```

---

## CLI

```rust
// src/main.rs

use clap::Parser;

#[derive(Parser)]
#[command(name = "sequencer")]
#[command(about = "PoA Sequencer with Celestia DA")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the sequencer
    Run {
        #[arg(short, long, default_value = "config.toml")]
        config: PathBuf,
    },

    /// Generate genesis file
    Genesis {
        #[arg(short, long)]
        config: PathBuf,

        #[arg(short, long)]
        output: PathBuf,
    },

    /// Generate validator key
    Keygen {
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { config } => {
            let config = Config::load(&config)?;
            let mut sequencer = Sequencer::new(config).await?;
            sequencer.run().await?;
        }
        Commands::Genesis { config, output } => {
            // Generate genesis.json
        }
        Commands::Keygen { output } => {
            // Generate validator keypair
        }
    }

    Ok(())
}
```

---

## Tasks

### 1. Project Structure
- [ ] Create `src/main.rs`
- [ ] Create `src/sequencer.rs`
- [ ] Create `src/config.rs`

### 2. Configuration
- [ ] Define Config struct
- [ ] TOML parsing
- [ ] Validation

### 3. Main Loop
- [ ] Wire consensus → execution
- [ ] Wire consensus → celestia
- [ ] Wire celestia finality → execution

### 4. CLI
- [ ] `run` command
- [ ] `genesis` command
- [ ] `keygen` command

### 5. Logging/Metrics
- [ ] Setup tracing
- [ ] Key events logged
- [ ] Prometheus metrics (optional)

### 6. Graceful Shutdown
- [ ] Signal handling (SIGTERM, SIGINT)
- [ ] Clean shutdown of all components

### 7. End-to-End Testing
- [ ] Single node test
- [ ] Multi-node test
- [ ] With actual reth

---

## Integration Test

```rust
#[tokio::test]
async fn test_end_to_end() {
    // 1. Start reth in dev mode
    let reth = start_reth_dev().await;

    // 2. Start sequencer
    let config = test_config(&reth);
    let mut sequencer = Sequencer::new(config).await.unwrap();

    // 3. Submit transaction
    let tx = create_test_tx();
    sequencer.consensus.submit_tx(tx).await.unwrap();

    // 4. Wait for block
    let block = timeout(
        Duration::from_secs(5),
        sequencer.consensus.block_stream().next()
    ).await.unwrap().unwrap();

    // 5. Verify execution
    assert!(block.transactions.len() > 0);

    // 6. Check reth state
    let balance = reth.get_balance(recipient).await.unwrap();
    assert!(balance > U256::ZERO);
}
```

---

## Output

When complete:

```bash
# Generate keys
./sequencer keygen --output /etc/sequencer/key

# Generate genesis
./sequencer genesis --config genesis-config.toml --output genesis.json

# Run sequencer
./sequencer run --config config.toml
```
