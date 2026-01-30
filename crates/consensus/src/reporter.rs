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
        self.handle_activity(&activity);
    }
}

impl ConsensusReporter {
    /// Handle consensus activity - log and forward finalization events.
    fn handle_activity(&mut self, activity: &Activity<Scheme, AppDigest>) {
        use Activity::*;
        match activity {
            Notarize(v) => tracing::debug!(round = ?v.round(), "Notarize vote"),
            Notarization(c) => tracing::debug!(view = ?c.view(), "Block notarized"),
            Certification(c) => tracing::debug!(view = ?c.view(), "Block certified"),
            Nullify(v) => tracing::debug!(round = ?v.round(), "Nullify vote"),
            Nullification(c) => tracing::debug!(view = ?c.view(), "View nullified"),
            Finalize(v) => tracing::debug!(round = ?v.round(), "Finalize vote"),
            Finalization(c) => {
                tracing::info!(view = ?c.view(), "Block finalized by consensus");
                self.mailbox.finalize(c.proposal.payload);
            }
            ConflictingNotarize(e) => tracing::warn!(signer = ?e.signer(), "Conflicting notarize"),
            ConflictingFinalize(e) => tracing::warn!(signer = ?e.signer(), "Conflicting finalize"),
            NullifyFinalize(e) => tracing::warn!(signer = ?e.signer(), "Nullify+Finalize"),
        }
    }
}
