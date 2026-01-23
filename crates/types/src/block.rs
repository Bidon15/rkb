//! Block types.

use alloy_primitives::{keccak256, Address, Bytes, B256};
use alloy_rlp::{Encodable, RlpEncodable};
use commonware_codec::{extensions::ReadExt as _, Write as CryptoWrite};
use commonware_cryptography::ed25519;
use serde::{Deserialize, Serialize};

use crate::serde_helpers::hex_fixed as hex_bytes;
use crate::Transaction;

/// Hash of a block.
pub type BlockHash = B256;

/// Block header containing metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, RlpEncodable)]
pub struct BlockHeader {
    /// Block height.
    pub height: u64,

    /// Timestamp (unix seconds).
    pub timestamp: u64,

    /// Parent block hash.
    pub parent_hash: BlockHash,

    /// Proposer address.
    pub proposer: Address,
}

/// A complete block with header, transactions, and execution results.
///
/// All blocks in the system are built via reth and contain the full
/// execution data. This ensures we fail fast at the source (reth integration)
/// rather than later with confusing None errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Block header.
    pub header: BlockHeader,

    /// Ordered transactions (RLP encoded).
    pub transactions: Vec<Transaction>,

    /// Validator signatures.
    pub signatures: Vec<Signature>,

    // === Reth-computed fields (required - blocks are always built via reth) ===

    /// State root computed by reth after executing transactions.
    pub state_root: B256,

    /// Receipts root computed by reth.
    pub receipts_root: B256,

    /// Logs bloom filter computed by reth.
    pub logs_bloom: Bytes,

    /// Previous RANDAO value used by reth.
    pub prev_randao: B256,

    /// Extra data used by reth.
    pub extra_data: Bytes,

    /// Gas used by all transactions (computed by reth).
    pub gas_used: u64,

    /// Gas limit for the block.
    pub gas_limit: u64,

    /// Base fee per gas.
    pub base_fee_per_gas: u64,

    /// The authoritative block hash computed by reth.
    pub block_hash: B256,
}

/// Validator signature on a block.
///
/// Contains both the Ethereum-style validator address and the Ed25519 public key
/// used for signing in the commonware consensus protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// Validator Ethereum address (derived from public key).
    pub validator: Address,

    /// Ed25519 public key bytes (32 bytes).
    /// Use [`Signature::public_key`] to get the typed public key.
    #[serde(with = "hex_bytes")]
    pub public_key_bytes: [u8; 32],

    /// Signature bytes (64 bytes for Ed25519).
    #[serde(with = "hex_bytes")]
    pub signature_bytes: [u8; 64],
}

impl Signature {
    /// Create a new signature from components.
    #[must_use]
    pub fn new(
        validator: Address,
        public_key: &ed25519::PublicKey,
        signature: &ed25519::Signature,
    ) -> Self {
        let mut pk_bytes = [0u8; 32];
        let mut pk_buf = &mut pk_bytes[..];
        public_key.write(&mut pk_buf);

        let mut sig_bytes = [0u8; 64];
        let mut sig_buf = &mut sig_bytes[..];
        signature.write(&mut sig_buf);

        Self {
            validator,
            public_key_bytes: pk_bytes,
            signature_bytes: sig_bytes,
        }
    }

    /// Get the Ed25519 public key.
    ///
    /// # Errors
    ///
    /// Returns `None` if the stored bytes are not a valid public key.
    #[must_use]
    pub fn public_key(&self) -> Option<ed25519::PublicKey> {
        let mut buf = &self.public_key_bytes[..];
        ed25519::PublicKey::read(&mut buf).ok()
    }

    /// Get the Ed25519 signature.
    ///
    /// # Errors
    ///
    /// Returns `None` if the stored bytes are not a valid signature.
    #[must_use]
    pub fn signature(&self) -> Option<ed25519::Signature> {
        let mut buf = &self.signature_bytes[..];
        ed25519::Signature::read(&mut buf).ok()
    }

    /// Verify this signature against a message.
    ///
    /// # Arguments
    ///
    /// * `namespace` - The signing namespace (for domain separation)
    /// * `message` - The message that was signed
    ///
    /// # Returns
    ///
    /// `true` if the signature is valid, `false` otherwise.
    #[must_use]
    pub fn verify(&self, namespace: &[u8], message: &[u8]) -> bool {
        use commonware_cryptography::Verifier;

        let Some(public_key) = self.public_key() else {
            return false;
        };
        let Some(signature) = self.signature() else {
            return false;
        };

        public_key.verify(namespace, message, &signature)
    }
}

/// Parameters for creating a new block from reth execution.
#[derive(Debug, Clone)]
pub struct BlockParams {
    /// Block header.
    pub header: BlockHeader,
    /// Transactions (RLP encoded).
    pub transactions: Vec<Transaction>,
    /// State root from reth.
    pub state_root: B256,
    /// Receipts root from reth.
    pub receipts_root: B256,
    /// Logs bloom from reth.
    pub logs_bloom: Bytes,
    /// Previous RANDAO value.
    pub prev_randao: B256,
    /// Extra data.
    pub extra_data: Bytes,
    /// Gas used.
    pub gas_used: u64,
    /// Gas limit.
    pub gas_limit: u64,
    /// Base fee per gas.
    pub base_fee_per_gas: u64,
    /// Block hash computed by reth.
    pub block_hash: B256,
}

impl Block {
    /// Create a new block with all required execution data.
    ///
    /// All blocks in the system must be created with complete reth execution data.
    /// This ensures we fail fast if reth integration is broken.
    #[must_use]
    pub fn new(params: BlockParams) -> Self {
        Self {
            header: params.header,
            transactions: params.transactions,
            signatures: Vec::new(),
            state_root: params.state_root,
            receipts_root: params.receipts_root,
            logs_bloom: params.logs_bloom,
            prev_randao: params.prev_randao,
            extra_data: params.extra_data,
            gas_used: params.gas_used,
            gas_limit: params.gas_limit,
            base_fee_per_gas: params.base_fee_per_gas,
            block_hash: params.block_hash,
        }
    }

    /// Compute the header hash (keccak256 of RLP-encoded header).
    ///
    /// Note: For the authoritative block hash, use `block_hash` field instead.
    /// This method computes a hash from our simplified header only.
    #[must_use]
    pub fn header_hash(&self) -> BlockHash {
        let mut buf = Vec::new();
        self.header.encode(&mut buf);
        keccak256(&buf)
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

    /// Parent block hash.
    #[must_use]
    pub fn parent_hash(&self) -> BlockHash {
        self.header.parent_hash
    }

    /// Number of transactions in this block.
    #[must_use]
    pub fn tx_count(&self) -> usize {
        self.transactions.len()
    }
}

// Test utilities - only available in test builds
#[cfg(any(test, feature = "test-utils"))]
impl Block {
    /// Create a test block with minimal required fields.
    ///
    /// **WARNING**: This is for unit tests only. Production code must create
    /// blocks through reth execution which provides all required fields.
    #[must_use]
    pub fn test_block(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        Self {
            header,
            transactions,
            signatures: Vec::new(),
            state_root: B256::ZERO,
            receipts_root: B256::ZERO,
            logs_bloom: Bytes::new(),
            prev_randao: B256::ZERO,
            extra_data: Bytes::new(),
            gas_used: 0,
            gas_limit: 30_000_000,
            base_fee_per_gas: 1_000_000_000, // 1 gwei
            block_hash: B256::ZERO,
        }
    }
}
