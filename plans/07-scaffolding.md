# Task 07: Repository Cleanup & Scaffolding

## Owner
First task before parallel work begins

## Goal
Remove Astria code and create clean, idiomatic Rust project structure for the new PoA sequencer.

---

## Phase 1: Cleanup (Delete Astria)

### Files to DELETE (everything except plans/)

```bash
# Delete all Astria code
rm -rf crates/
rm -rf proto/
rm -rf specs/
rm -rf charts/
rm -rf scripts/
rm -rf tools/
rm -rf .github/
rm -rf docker/

# Delete config files
rm -f Cargo.toml
rm -f Cargo.lock
rm -f rust-toolchain.toml
rm -f .cargo/
rm -f deny.toml
rm -f justfile
rm -f Makefile

# Keep
# - plans/          (our planning docs)
# - .git/           (preserve history for reference)
# - LICENSE         (update if needed)
# - README.md       (will rewrite)
```

### Verify Clean State

```bash
ls -la
# Should only show:
# .git/
# plans/
# LICENSE
# README.md (to be rewritten)
```

---

## Phase 2: New Project Structure

### Directory Layout

```
sequencer/
├── Cargo.toml              # Workspace root
├── Cargo.lock
├── rust-toolchain.toml
├── .cargo/
│   └── config.toml
├── deny.toml               # cargo-deny config
├── clippy.toml             # clippy config
├── rustfmt.toml            # rustfmt config
│
├── crates/
│   ├── sequencer/          # Main binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── cli.rs
│   │       ├── config.rs
│   │       └── sequencer.rs
│   │
│   ├── consensus/          # PoA consensus module
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── engine.rs
│   │       ├── block.rs
│   │       └── validator.rs
│   │
│   ├── celestia/           # Celestia DA module
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── client.rs
│   │       ├── blob.rs
│   │       └── finality.rs
│   │
│   ├── execution/          # Reth Engine API client
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── client.rs
│   │       ├── payload.rs
│   │       └── forkchoice.rs
│   │
│   └── types/              # Shared types
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── block.rs
│           ├── transaction.rs
│           └── config.rs
│
├── contracts/              # Solidity contracts
│   ├── foundry.toml
│   ├── src/
│   │   └── IOURegistry.sol
│   └── test/
│       └── IOURegistry.t.sol
│
├── docs/
│   ├── quickstart.md
│   ├── configuration.md
│   ├── validator-guide.md
│   └── architecture.md
│
├── plans/                  # Planning docs (already exists)
│
├── scripts/
│   ├── init.sh
│   ├── keygen.sh
│   └── genesis.sh
│
├── docker/
│   ├── Dockerfile
│   └── docker-compose.yml
│
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── release.yml
│
├── LICENSE
└── README.md
```

---

## Phase 3: Workspace Configuration

### Cargo.toml (workspace root)

```toml
[workspace]
resolver = "2"
members = [
    "crates/sequencer",
    "crates/consensus",
    "crates/celestia",
    "crates/execution",
    "crates/types",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.83"
license = "MIT OR Apache-2.0"
repository = "https://github.com/your-org/sequencer"

[workspace.dependencies]
# Internal crates
sequencer-consensus = { path = "crates/consensus" }
sequencer-celestia = { path = "crates/celestia" }
sequencer-execution = { path = "crates/execution" }
sequencer-types = { path = "crates/types" }

# Commonware
commonware-consensus = "0.0.65"
commonware-p2p = "0.0.65"
commonware-cryptography = "0.0.65"
commonware-runtime = "0.0.65"
commonware-storage = "0.0.65"

# Celestia / Lumina
lumina-node = "0.5"
celestia-types = "0.7"

# Ethereum / Execution
alloy-primitives = "0.8"
alloy-rlp = "0.3"
alloy-rpc-types-engine = "0.3"
alloy-consensus = "0.3"

# Async
tokio = { version = "1.43", features = ["full"] }
futures = "0.3"
async-trait = "0.1"

# RPC
jsonrpsee = { version = "0.24", features = ["http-client", "server"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
bincode = "1"

# Error handling
thiserror = "2"
anyhow = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Crypto
rand = "0.8"
hex = "0.4"

# Metrics
metrics = "0.24"
metrics-exporter-prometheus = "0.16"

# Testing
proptest = "1"
rstest = "0.23"

[workspace.lints.rust]
unsafe_code = "deny"
missing_docs = "warn"
rust_2018_idioms = "warn"

[workspace.lints.clippy]
all = "warn"
pedantic = "warn"
nursery = "warn"
cargo = "warn"
# Allow some pedantic lints
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"

[profile.release]
lto = "thin"
strip = true
codegen-units = 1
panic = "abort"

[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3
```

### rust-toolchain.toml

```toml
[toolchain]
channel = "1.83"
components = ["rustfmt", "clippy", "rust-analyzer"]
```

### .cargo/config.toml

```toml
[build]
rustflags = ["-C", "target-cpu=native"]

[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

[alias]
xtask = "run --package xtask --"
```

### rustfmt.toml

```toml
edition = "2024"
max_width = 100
use_small_heuristics = "Max"
imports_granularity = "Module"
group_imports = "StdExternalCrate"
reorder_imports = true
```

### clippy.toml

```toml
cognitive-complexity-threshold = 15
too-many-arguments-threshold = 8
type-complexity-threshold = 300
```

### deny.toml

```toml
[advisories]
vulnerability = "deny"
unmaintained = "warn"
yanked = "deny"
notice = "warn"

[licenses]
unlicensed = "deny"
allow = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "CC0-1.0",
    "Unlicense",
]
copyleft = "warn"

[bans]
multiple-versions = "warn"
wildcards = "deny"
deny = [
    # Use parking_lot instead
    { name = "lazy_static" },
]

[sources]
unknown-registry = "deny"
unknown-git = "warn"
```

---

## Phase 4: Crate Scaffolding

### crates/types/Cargo.toml

```toml
[package]
name = "sequencer-types"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
alloy-primitives.workspace = true
serde.workspace = true
thiserror.workspace = true
```

### crates/types/src/lib.rs

```rust
//! Core types for the PoA sequencer.
//!
//! This crate provides shared type definitions used across all sequencer components.

#![warn(missing_docs)]

mod block;
mod config;
mod transaction;

pub use block::{Block, BlockHeader, BlockHash};
pub use config::{ChainConfig, GenesisConfig};
pub use transaction::{Transaction, TransactionHash};

/// Re-export commonly used types from alloy.
pub mod primitives {
    pub use alloy_primitives::{Address, B256, U256};
}
```

### crates/types/src/block.rs

```rust
//! Block types.

use alloy_primitives::B256;
use serde::{Deserialize, Serialize};

use crate::Transaction;

/// Hash of a block.
pub type BlockHash = B256;

/// Block header containing metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    /// Block height.
    pub height: u64,

    /// Timestamp (unix seconds).
    pub timestamp: u64,

    /// Parent block hash.
    pub parent_hash: BlockHash,

    /// Proposer address.
    pub proposer: alloy_primitives::Address,
}

/// A complete block with header and transactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Block header.
    pub header: BlockHeader,

    /// Ordered transactions.
    pub transactions: Vec<Transaction>,

    /// Validator signatures.
    pub signatures: Vec<Signature>,
}

/// Validator signature on a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    /// Validator public key.
    pub validator: alloy_primitives::Address,

    /// Signature bytes.
    pub signature: Vec<u8>,
}

impl Block {
    /// Compute the block hash.
    #[must_use]
    pub fn hash(&self) -> BlockHash {
        // TODO: Implement proper hashing
        B256::ZERO
    }

    /// Block height.
    #[must_use]
    pub fn height(&self) -> u64 {
        self.header.height
    }

    /// Block timestamp.
    #[must_use]
    pub fn timestamp(&self) -> u64 {
        self.header.timestamp
    }
}
```

### crates/types/src/transaction.rs

```rust
//! Transaction types.

use alloy_primitives::B256;
use serde::{Deserialize, Serialize};

/// Hash of a transaction.
pub type TransactionHash = B256;

/// A transaction to be included in a block.
///
/// Transactions are opaque bytes - the execution layer (reth) interprets them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    /// Raw transaction bytes (RLP-encoded Ethereum transaction).
    data: Vec<u8>,
}

impl Transaction {
    /// Create a new transaction from raw bytes.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Get the raw transaction bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Consume and return the raw bytes.
    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Compute the transaction hash.
    #[must_use]
    pub fn hash(&self) -> TransactionHash {
        // TODO: Implement proper keccak256 hashing
        B256::ZERO
    }
}
```

### crates/consensus/Cargo.toml

```toml
[package]
name = "sequencer-consensus"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
sequencer-types.workspace = true

commonware-consensus.workspace = true
commonware-p2p.workspace = true
commonware-cryptography.workspace = true
commonware-runtime.workspace = true

tokio.workspace = true
futures.workspace = true
async-trait.workspace = true
tracing.workspace = true
thiserror.workspace = true

[dev-dependencies]
rstest.workspace = true
```

### crates/consensus/src/lib.rs

```rust
//! PoA consensus module using commonware-consensus.
//!
//! This crate provides block production and ordering for the sequencer.

#![warn(missing_docs)]

mod engine;
mod error;
mod validator;

pub use engine::{Consensus, ConsensusConfig};
pub use error::ConsensusError;
pub use validator::ValidatorSet;

use futures::Stream;
use sequencer_types::Block;

/// Result type for consensus operations.
pub type Result<T> = std::result::Result<T, ConsensusError>;

/// Trait for consensus block production.
#[async_trait::async_trait]
pub trait BlockProducer: Send + Sync {
    /// Start the consensus engine.
    async fn start(&self) -> Result<()>;

    /// Submit a transaction to the mempool.
    async fn submit_transaction(&self, tx: sequencer_types::Transaction) -> Result<()>;

    /// Get a stream of finalized blocks.
    fn block_stream(&self) -> impl Stream<Item = Block> + Send;
}
```

### crates/celestia/Cargo.toml

```toml
[package]
name = "sequencer-celestia"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
sequencer-types.workspace = true

lumina-node.workspace = true
celestia-types.workspace = true

tokio.workspace = true
futures.workspace = true
async-trait.workspace = true
tracing.workspace = true
thiserror.workspace = true
bincode.workspace = true

[dev-dependencies]
rstest.workspace = true
```

### crates/celestia/src/lib.rs

```rust
//! Celestia DA module using Lumina client.
//!
//! This crate handles blob submission and finality tracking.

#![warn(missing_docs)]

mod blob;
mod client;
mod error;
mod finality;

pub use client::{CelestiaClient, CelestiaConfig};
pub use error::CelestiaError;
pub use finality::{FinalityConfirmation, FinalityTracker};

use alloy_primitives::B256;
use futures::Stream;

/// Result type for Celestia operations.
pub type Result<T> = std::result::Result<T, CelestiaError>;

/// Information about a submitted blob.
#[derive(Debug, Clone)]
pub struct BlobSubmission {
    /// Our block hash.
    pub block_hash: B256,

    /// Celestia block height where blob was included.
    pub celestia_height: u64,

    /// Blob commitment for verification.
    pub commitment: Vec<u8>,
}

/// Trait for Celestia DA operations.
#[async_trait::async_trait]
pub trait DataAvailability: Send + Sync {
    /// Submit a block as a blob.
    async fn submit_block(&self, block: &sequencer_types::Block) -> Result<BlobSubmission>;

    /// Get finality confirmation stream.
    fn finality_stream(&self) -> impl Stream<Item = FinalityConfirmation> + Send;
}
```

### crates/execution/Cargo.toml

```toml
[package]
name = "sequencer-execution"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
sequencer-types.workspace = true

alloy-primitives.workspace = true
alloy-rlp.workspace = true
alloy-rpc-types-engine.workspace = true
alloy-consensus.workspace = true

jsonrpsee.workspace = true
tokio.workspace = true
tracing.workspace = true
thiserror.workspace = true
hex.workspace = true

[dev-dependencies]
rstest.workspace = true
```

### crates/execution/src/lib.rs

```rust
//! Execution module using reth Engine API.
//!
//! This crate drives block execution on a vanilla reth node.

#![warn(missing_docs)]

mod client;
mod error;
mod forkchoice;
mod payload;

pub use client::{ExecutionClient, ExecutionConfig};
pub use error::ExecutionError;
pub use forkchoice::ForkchoiceState;
pub use payload::ExecutionResult;

use alloy_primitives::B256;

/// Result type for execution operations.
pub type Result<T> = std::result::Result<T, ExecutionError>;

/// Trait for execution layer operations.
#[async_trait::async_trait]
pub trait Execution: Send + Sync {
    /// Execute a block and return the result.
    async fn execute_block(&self, block: &sequencer_types::Block) -> Result<ExecutionResult>;

    /// Update forkchoice (soft/firm confirmations).
    async fn update_forkchoice(&self, state: ForkchoiceState) -> Result<()>;

    /// Get current forkchoice state.
    async fn forkchoice(&self) -> Result<ForkchoiceState>;
}
```

### crates/sequencer/Cargo.toml

```toml
[package]
name = "sequencer"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "sequencer"
path = "src/main.rs"

[dependencies]
sequencer-types.workspace = true
sequencer-consensus.workspace = true
sequencer-celestia.workspace = true
sequencer-execution.workspace = true

tokio.workspace = true
futures.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap.workspace = true
serde.workspace = true
toml.workspace = true
anyhow.workspace = true
metrics.workspace = true
metrics-exporter-prometheus.workspace = true
```

### crates/sequencer/src/main.rs

```rust
//! PoA Sequencer with Celestia DA.
//!
//! A minimal sequencer for PoA EVM chains using Celestia for data availability.

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod cli;
mod config;
mod sequencer;

use cli::{Cli, Commands};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Parse CLI
    let cli = Cli::parse();

    // Run command
    match cli.command {
        Commands::Run { config } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;

            rt.block_on(async {
                let config = config::Config::load(&config)?;
                let mut sequencer = sequencer::Sequencer::new(config).await?;
                sequencer.run().await
            })
        }

        Commands::Genesis { config, output } => {
            todo!("Generate genesis file")
        }

        Commands::Keygen { output } => {
            todo!("Generate validator key")
        }
    }
}
```

### crates/sequencer/src/cli.rs

```rust
//! CLI argument parsing.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// PoA Sequencer with Celestia DA.
#[derive(Parser)]
#[command(name = "sequencer")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Command to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// Available commands.
#[derive(Subcommand)]
pub enum Commands {
    /// Run the sequencer.
    Run {
        /// Path to configuration file.
        #[arg(short, long, default_value = "config.toml")]
        config: PathBuf,
    },

    /// Generate genesis file.
    Genesis {
        /// Path to genesis configuration.
        #[arg(short, long)]
        config: PathBuf,

        /// Output path for genesis.json.
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Generate validator keypair.
    Keygen {
        /// Output path for key file.
        #[arg(short, long)]
        output: PathBuf,
    },
}
```

---

## Phase 5: Initial Files

### README.md

```markdown
# Sequencer

A minimal PoA sequencer for EVM chains with Celestia DA.

## Features

- **PoA Consensus**: Fast block production with configurable validator set
- **Celestia DA**: Data availability and finality from Celestia
- **Vanilla Reth**: No fork required, uses standard Engine API
- **Two-tier Finality**: Soft (instant) + Firm (Celestia-backed)

## Quick Start

```bash
# Initialize
./scripts/init.sh

# Configure
vim config.toml

# Run
docker-compose up -d
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  SEQUENCER                                                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
│  │ Consensus   │  │ Celestia    │  │ Execution   │              │
│  │ (PoA)       │  │ (Lumina)    │  │ (Engine API)│              │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘              │
└─────────┼────────────────┼────────────────┼─────────────────────┘
          │                │                │
          ▼                ▼                ▼
     commonware       Celestia         Vanilla Reth
```

## Documentation

- [Quickstart](docs/quickstart.md)
- [Configuration](docs/configuration.md)
- [Validator Guide](docs/validator-guide.md)
- [Architecture](docs/architecture.md)

## License

MIT OR Apache-2.0
```

### LICENSE

Keep existing or update to:

```
MIT OR Apache-2.0
```

---

## Phase 6: Verification

### Build Check

```bash
cargo check --workspace
cargo clippy --workspace
cargo fmt --check
cargo deny check
```

### Test Scaffold

```bash
cargo test --workspace
```

---

## Execution Order

```bash
# 1. Clean
rm -rf crates/ proto/ specs/ charts/ scripts/ tools/ docker/ .github/
rm -f Cargo.toml Cargo.lock rust-toolchain.toml deny.toml justfile Makefile

# 2. Create structure
mkdir -p crates/{sequencer,consensus,celestia,execution,types}/src
mkdir -p contracts/{src,test}
mkdir -p docs scripts docker .github/workflows .cargo

# 3. Write config files
# (Cargo.toml, rust-toolchain.toml, etc.)

# 4. Write crate scaffolds
# (lib.rs, mod.rs files)

# 5. Verify
cargo check --workspace
```

---

## Checklist

- [ ] Delete all Astria code
- [ ] Create workspace Cargo.toml
- [ ] Create rust-toolchain.toml
- [ ] Create .cargo/config.toml
- [ ] Create rustfmt.toml
- [ ] Create clippy.toml
- [ ] Create deny.toml
- [ ] Scaffold crates/types
- [ ] Scaffold crates/consensus
- [ ] Scaffold crates/celestia
- [ ] Scaffold crates/execution
- [ ] Scaffold crates/sequencer
- [ ] Create README.md
- [ ] Create docs/ stubs
- [ ] Create scripts/ stubs
- [ ] Create docker/ stubs
- [ ] Create .github/workflows
- [ ] Verify `cargo check --workspace`
- [ ] Verify `cargo clippy --workspace`
