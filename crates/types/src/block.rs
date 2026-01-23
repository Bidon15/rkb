//! Block types.

use alloy_primitives::{keccak256, Address, Bytes, B256};
use alloy_rlp::{Encodable, RlpEncodable};
use commonware_codec::{extensions::ReadExt as _, Write as CryptoWrite};
use commonware_cryptography::ed25519;
use serde::{Deserialize, Serialize};

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

/// A complete block with header and transactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Block header.
    pub header: BlockHeader,

    /// Ordered transactions.
    pub transactions: Vec<Transaction>,

    /// Validator signatures.
    pub signatures: Vec<Signature>,

    // === Reth-computed fields (set when built via vanilla Ethereum flow) ===
    // These fields are populated by the proposer from reth's BuiltPayload.
    // They're needed so validators can call newPayloadV3 with the correct values.

    /// State root computed by reth after executing transactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_root: Option<B256>,

    /// Receipts root computed by reth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipts_root: Option<B256>,

    /// Logs bloom filter computed by reth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs_bloom: Option<Bytes>,

    /// Previous RANDAO value used by reth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_randao: Option<B256>,

    /// Extra data used by reth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_data: Option<Bytes>,

    /// Gas used by all transactions (computed by reth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_used: Option<u64>,

    /// Gas limit for the block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,

    /// Base fee per gas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_fee_per_gas: Option<u64>,

    /// The authoritative block hash computed by reth.
    /// This is different from `hash()` which computes from our header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reth_block_hash: Option<B256>,
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

/// Serde helper for hex-encoded fixed-size byte arrays.
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S, const N: usize>(bytes: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<[u8; N], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        let len = bytes.len();
        bytes.try_into().map_err(|_| {
            serde::de::Error::custom(format!("expected {N} bytes, got {len}"))
        })
    }
}

impl Block {
    /// Create a new block.
    #[must_use]
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        Self {
            header,
            transactions,
            signatures: Vec::new(),
            state_root: None,
            receipts_root: None,
            logs_bloom: None,
            prev_randao: None,
            extra_data: None,
            gas_used: None,
            gas_limit: None,
            base_fee_per_gas: None,
            reth_block_hash: None,
        }
    }

    /// Set the reth-computed fields from a built payload.
    ///
    /// This should be called by the proposer after building a block via reth.
    pub fn set_reth_fields(
        &mut self,
        state_root: B256,
        receipts_root: B256,
        logs_bloom: Bytes,
        prev_randao: B256,
        extra_data: Bytes,
        gas_used: u64,
        gas_limit: u64,
        base_fee_per_gas: u64,
        reth_block_hash: B256,
    ) {
        self.state_root = Some(state_root);
        self.receipts_root = Some(receipts_root);
        self.logs_bloom = Some(logs_bloom);
        self.prev_randao = Some(prev_randao);
        self.extra_data = Some(extra_data);
        self.gas_used = Some(gas_used);
        self.gas_limit = Some(gas_limit);
        self.base_fee_per_gas = Some(base_fee_per_gas);
        self.reth_block_hash = Some(reth_block_hash);
    }

    /// Check if this block has reth-computed fields populated.
    #[must_use]
    pub fn has_reth_fields(&self) -> bool {
        self.state_root.is_some()
    }

    /// Compute the block hash (keccak256 of RLP-encoded header).
    ///
    /// Note: This performs RLP encoding and cryptographic hash computation on each call.
    /// Cache the result if calling multiple times.
    #[must_use]
    pub fn hash(&self) -> BlockHash {
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
