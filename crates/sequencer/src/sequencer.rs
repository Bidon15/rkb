//! Main sequencer orchestration loop.

use std::sync::Arc;

use eyre::Result;
use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use sequencer_celestia::{CelestiaClient, DataAvailability, FinalityConfirmation};
use sequencer_consensus::{BlockProducer, Consensus};
use sequencer_execution::ExecutionClient;

use crate::config::Config;

/// The main sequencer node.
pub struct Sequencer {
    config: Config,
    consensus: Consensus,
    celestia: Arc<CelestiaClient>,
    execution: ExecutionClient,
    finality_rx: mpsc::Receiver<FinalityConfirmation>,
}

impl Sequencer {
    /// Create a new sequencer instance.
    ///
    /// # Errors
    ///
    /// Returns an error if any component fails to initialize.
    pub async fn new(config: Config) -> Result<Self> {
        // Initialize consensus
        let consensus = Consensus::new(config.consensus.clone())?;

        // Create finality channel for Celestia confirmations
        let (finality_tx, finality_rx) = mpsc::channel(100);

        // Initialize Celestia client
        let celestia = Arc::new(CelestiaClient::new(config.celestia.clone(), finality_tx).await?);

        // Initialize execution client
        let execution = ExecutionClient::new(config.execution.clone())?;

        Ok(Self {
            config,
            consensus,
            celestia,
            execution,
            finality_rx,
        })
    }

    /// Run the sequencer main loop.
    ///
    /// # Errors
    ///
    /// Returns an error if the sequencer encounters a fatal error.
    pub async fn run(mut self) -> Result<()> {
        info!(
            chain_id = self.config.chain_id,
            "Sequencer starting"
        );

        // Start consensus engine
        self.consensus.start().await?;
        info!("Consensus engine started");

        // Start Celestia finality tracker in background
        let celestia_clone = Arc::clone(&self.celestia);
        tokio::spawn(async move {
            if let Err(e) = celestia_clone.run_finality_tracker().await {
                error!(error = %e, "Celestia finality tracker failed");
            }
        });
        info!("Celestia finality tracker started");

        // Get block stream from consensus
        let mut block_stream = self.consensus.block_stream();

        // Main event loop
        info!("Entering main event loop");

        loop {
            tokio::select! {
                // Handle new blocks from consensus
                Some(block) = block_stream.next() => {
                    let block_height = block.height();
                    let block_hash = block.hash();
                    let tx_count = block.tx_count();

                    info!(
                        block_height,
                        %block_hash,
                        tx_count,
                        "New block from consensus"
                    );

                    // Submit block to Celestia
                    match self.celestia.submit_block(&block).await {
                        Ok(submission) => {
                            info!(
                                block_height,
                                celestia_height = submission.celestia_height,
                                "Block submitted to Celestia"
                            );

                            // Track finality for this block
                            self.celestia.track_finality(block_hash, submission);
                        }
                        Err(e) => {
                            error!(
                                block_height,
                                error = %e,
                                "Failed to submit block to Celestia"
                            );
                            // Continue processing - block is still valid locally
                        }
                    }
                }

                // Handle finality confirmations from Celestia
                Some(confirmation) = self.finality_rx.recv() => {
                    info!(
                        block_height = confirmation.block_height,
                        celestia_height = confirmation.celestia_height,
                        latency_ms = confirmation.latency_ms(),
                        "Block finalized on Celestia"
                    );

                    // TODO: Update execution layer finalized block
                    // self.execution.update_finalized(confirmation.block_hash).await?;

                    debug!(
                        pending_count = self.celestia.pending_finality_count().await,
                        "Pending finality count"
                    );
                }

                // Handle shutdown signal
                _ = tokio::signal::ctrl_c() => {
                    warn!("Shutdown signal received");
                    break;
                }
            }
        }

        info!("Sequencer shutting down");
        Ok(())
    }
}
