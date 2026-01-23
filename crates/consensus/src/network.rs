//! P2P networking layer using commonware-p2p.
//!
//! This module provides authenticated peer-to-peer communication for the
//! consensus protocol using the commonware-p2p library.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use commonware_cryptography::ed25519;
use commonware_p2p::{authenticated::discovery, Ingress};
use commonware_utils::union;

/// P2P network configuration.
#[derive(Clone)]
pub struct NetworkConfig {
    /// Our signing key.
    pub signer: ed25519::PrivateKey,

    /// Namespace for message signing.
    pub namespace: Vec<u8>,

    /// Local listen address.
    pub listen_addr: SocketAddr,

    /// Dialable address (how other peers can reach us).
    pub dialable_addr: SocketAddr,

    /// Bootstrapper peers (public_key, socket_addr).
    pub bootstrappers: Vec<(ed25519::PublicKey, SocketAddr)>,

    /// Maximum message size.
    pub max_message_size: u32,

    /// Allow private IPs (for local testing).
    pub allow_private_ips: bool,
}

impl NetworkConfig {
    /// Create a configuration for local development/testing.
    pub fn local(
        signer: ed25519::PrivateKey,
        namespace: &[u8],
        port: u16,
        bootstrappers: Vec<(ed25519::PublicKey, SocketAddr)>,
    ) -> Self {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        Self {
            signer,
            namespace: namespace.to_vec(),
            listen_addr: addr,
            dialable_addr: addr,
            bootstrappers,
            max_message_size: 1024 * 1024, // 1MB
            allow_private_ips: true,
        }
    }

    /// Create a configuration for production.
    pub fn production(
        signer: ed25519::PrivateKey,
        namespace: &[u8],
        listen_addr: SocketAddr,
        dialable_addr: SocketAddr,
        bootstrappers: Vec<(ed25519::PublicKey, SocketAddr)>,
    ) -> Self {
        Self {
            signer,
            namespace: namespace.to_vec(),
            listen_addr,
            dialable_addr,
            bootstrappers,
            max_message_size: 1024 * 1024, // 1MB
            allow_private_ips: false,
        }
    }

    /// Convert to commonware-p2p discovery config.
    pub fn to_discovery_config(&self) -> discovery::Config<ed25519::PrivateKey> {
        // Convert SocketAddr bootstrappers to Ingress
        let bootstrappers: Vec<_> = self
            .bootstrappers
            .iter()
            .map(|(pk, addr)| (pk.clone(), Ingress::from(*addr)))
            .collect();

        let mut cfg = if self.allow_private_ips {
            discovery::Config::local(
                self.signer.clone(),
                &union(&self.namespace, b"_P2P"),
                self.listen_addr,
                self.dialable_addr,
                bootstrappers,
                self.max_message_size,
            )
        } else {
            discovery::Config::recommended(
                self.signer.clone(),
                &union(&self.namespace, b"_P2P"),
                self.listen_addr,
                self.dialable_addr,
                bootstrappers,
                self.max_message_size,
            )
        };

        // Adjust for faster discovery in development
        if self.allow_private_ips {
            cfg.dial_frequency = Duration::from_millis(500);
            cfg.query_frequency = Duration::from_secs(10);
        }

        cfg
    }
}

/// Channel identifiers for consensus messages.
pub mod channels {
    /// Channel for individual votes (Notarize, Nullify, Finalize).
    pub const VOTE: u32 = 0;

    /// Channel for certificates (Notarization, Nullification, Finalization).
    pub const CERTIFICATE: u32 = 1;

    /// Channel for resolver requests (certificate fetching).
    pub const RESOLVER: u32 = 2;

    /// Channel for block relay (application-level block propagation).
    pub const BLOCK_RELAY: u32 = 3;
}

/// Rate limits for consensus channels.
pub mod rate_limits {
    use commonware_runtime::Quota;

    /// Rate limit for vote messages.
    pub fn vote() -> Quota {
        Quota::per_second(commonware_utils::NZU32!(100))
    }

    /// Rate limit for certificate messages.
    pub fn certificate() -> Quota {
        Quota::per_second(commonware_utils::NZU32!(50))
    }

    /// Rate limit for resolver messages.
    pub fn resolver() -> Quota {
        Quota::per_second(commonware_utils::NZU32!(20))
    }

    /// Rate limit for block relay messages.
    pub fn block_relay() -> Quota {
        Quota::per_second(commonware_utils::NZU32!(10))
    }
}

/// Message backlog sizes for channels.
pub mod backlog {
    /// Backlog for vote messages.
    pub const VOTE: usize = 512;

    /// Backlog for certificate messages.
    pub const CERTIFICATE: usize = 256;

    /// Backlog for resolver messages.
    pub const RESOLVER: usize = 128;

    /// Backlog for block relay messages.
    pub const BLOCK_RELAY: usize = 64;
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::Signer;

    #[test]
    fn test_local_config() {
        let signer = ed25519::PrivateKey::from_seed(0);
        let cfg = NetworkConfig::local(signer, b"test", 26656, vec![]);

        assert!(cfg.allow_private_ips);
        assert_eq!(cfg.max_message_size, 1024 * 1024);
    }

    #[test]
    fn test_production_config() {
        let signer = ed25519::PrivateKey::from_seed(0);
        let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 26656);
        let dialable = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 26656);

        let cfg = NetworkConfig::production(signer, b"test", listen, dialable, vec![]);

        assert!(!cfg.allow_private_ips);
    }
}
