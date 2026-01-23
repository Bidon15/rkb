//! Reporter implementation for Simplex consensus.
//!
//! Reports consensus activity for monitoring and slashing.

use commonware_consensus::{
    simplex::{
        scheme::ed25519::Scheme,
        types::{Activity, Attributable},
    },
    Reporter, Viewable,
};

use crate::application::{AppDigest, Mailbox};

/// Reporter that logs consensus activity and forwards finalization events.
///
/// When a block is finalized by consensus, the reporter extracts the payload
/// digest and sends a finalize message to the application actor, which then
/// retrieves the block from pending storage and broadcasts it.
#[derive(Clone)]
pub struct ConsensusReporter {
    /// Whether to log activity at trace level.
    verbose: bool,
    /// Mailbox to send finalization events to the application.
    mailbox: Mailbox,
}

impl ConsensusReporter {
    /// Create a new reporter with a mailbox for forwarding finalization events.
    pub fn new(verbose: bool, mailbox: Mailbox) -> Self {
        Self { verbose, mailbox }
    }
}

impl Reporter for ConsensusReporter {
    type Activity = Activity<Scheme, AppDigest>;

    async fn report(&mut self, activity: Self::Activity) {
        if self.verbose {
            tracing::trace!(?activity, "Consensus activity");
        }

        // Track activity for monitoring
        match &activity {
            Activity::Notarize(vote) => {
                tracing::debug!(round = ?vote.round(), "Notarize vote");
            }
            Activity::Notarization(cert) => {
                tracing::debug!(view = ?cert.view(), "Block notarized");
            }
            Activity::Certification(cert) => {
                tracing::debug!(view = ?cert.view(), "Block certified");
            }
            Activity::Nullify(vote) => {
                tracing::debug!(round = ?vote.round(), "Nullify vote");
            }
            Activity::Nullification(cert) => {
                tracing::debug!(view = ?cert.view(), "View nullified");
            }
            Activity::Finalize(vote) => {
                tracing::debug!(round = ?vote.round(), "Finalize vote");
            }
            Activity::Finalization(cert) => {
                tracing::info!(view = ?cert.view(), "Block finalized by consensus");
                // Forward finalization to application actor to retrieve and broadcast the block
                let digest = cert.proposal.payload;
                self.mailbox.finalize(digest);
            }
            Activity::ConflictingNotarize(evidence) => {
                tracing::warn!(signer = ?evidence.signer(), "Conflicting notarize detected (equivocation)");
            }
            Activity::ConflictingFinalize(evidence) => {
                tracing::warn!(signer = ?evidence.signer(), "Conflicting finalize detected (equivocation)");
            }
            Activity::NullifyFinalize(evidence) => {
                tracing::warn!(signer = ?evidence.signer(), "Nullify+Finalize detected (equivocation)");
            }
        }
    }
}
