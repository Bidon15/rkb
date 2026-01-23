//! Block index for mapping sequencer blocks to Celestia DA locations.
//!
//! This module provides a persistent index that tracks where each sequencer block
//! is stored in Celestia. The index supports lookups by both block height and hash.
//!
//! # Storage Format
//!
//! The index is stored as an append-only file of newline-delimited JSON entries.
//! On startup, the file is replayed to rebuild the in-memory index.
//!
//! # Future: Migration to commonware-storage
//!
//! This implementation uses simple file I/O for compatibility with tokio.
//! Once the sequencer architecture is restructured to run within the commonware
//! runtime, this can be migrated to `commonware-storage::archive::immutable::Archive`
//! for better performance and crash recovery guarantees.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, info, warn};

use crate::error::CelestiaError;
use crate::Result;

/// Entry in the block index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Sequencer block height.
    pub block_height: u64,
    /// Sequencer block hash.
    pub block_hash: B256,
    /// Celestia height where the block was posted.
    pub celestia_height: u64,
    /// Blob commitment for verification.
    #[serde(with = "hex_bytes")]
    pub commitment: Vec<u8>,
}

/// Persistent index mapping sequencer blocks to Celestia locations.
///
/// Supports efficient lookups by block height or hash.
pub struct BlockIndex {
    /// Path to the index file.
    path: PathBuf,
    /// In-memory index by block height.
    by_height: BTreeMap<u64, IndexEntry>,
    /// In-memory index: block hash -> block height.
    by_hash: HashMap<B256, u64>,
    /// File handle for appending new entries.
    file: Option<File>,
}

impl BlockIndex {
    /// Open or create a block index at the given path.
    ///
    /// If the index file exists, it will be replayed to rebuild the in-memory state.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                CelestiaError::Internal(format!("failed to create index directory: {e}"))
            })?;
        }

        let mut index = Self {
            path: path.clone(),
            by_height: BTreeMap::new(),
            by_hash: HashMap::new(),
            file: None,
        };

        // Replay existing entries if file exists
        if path.exists() {
            index.replay().await?;
        }

        // Open file for appending
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| CelestiaError::Internal(format!("failed to open index file: {e}")))?;

        index.file = Some(file);

        info!(
            path = %path.display(),
            entries = index.by_height.len(),
            "Block index opened"
        );

        Ok(index)
    }

    /// Replay the index file to rebuild in-memory state.
    async fn replay(&mut self) -> Result<()> {
        let file = File::open(&self.path).await.map_err(|e| {
            CelestiaError::Internal(format!("failed to open index for replay: {e}"))
        })?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut count = 0;
        let mut errors = 0;

        while let Some(line) = lines.next_line().await.map_err(|e| {
            CelestiaError::Internal(format!("failed to read index line: {e}"))
        })? {
            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<IndexEntry>(&line) {
                Ok(entry) => {
                    self.by_hash.insert(entry.block_hash, entry.block_height);
                    self.by_height.insert(entry.block_height, entry);
                    count += 1;
                }
                Err(e) => {
                    warn!(error = %e, line = %line, "Skipping malformed index entry");
                    errors += 1;
                }
            }
        }

        debug!(entries = count, errors, "Index replay complete");
        Ok(())
    }

    /// Insert a new entry into the index.
    ///
    /// The entry is appended to the file and added to the in-memory index.
    pub async fn insert(&mut self, entry: IndexEntry) -> Result<()> {
        // Serialize and append to file
        let mut line = serde_json::to_string(&entry).map_err(|e| {
            CelestiaError::Internal(format!("failed to serialize index entry: {e}"))
        })?;
        line.push('\n');

        if let Some(ref mut file) = self.file {
            file.write_all(line.as_bytes()).await.map_err(|e| {
                CelestiaError::Internal(format!("failed to write index entry: {e}"))
            })?;
            file.flush().await.map_err(|e| {
                CelestiaError::Internal(format!("failed to flush index: {e}"))
            })?;
        }

        // Update in-memory index
        self.by_hash.insert(entry.block_hash, entry.block_height);
        self.by_height.insert(entry.block_height, entry);

        Ok(())
    }

    /// Get an entry by block height.
    pub fn get_by_height(&self, height: u64) -> Option<&IndexEntry> {
        self.by_height.get(&height)
    }

    /// Get an entry by block hash.
    pub fn get_by_hash(&self, hash: &B256) -> Option<&IndexEntry> {
        self.by_hash
            .get(hash)
            .and_then(|height| self.by_height.get(height))
    }

    /// Get the highest indexed block height.
    pub fn latest_height(&self) -> Option<u64> {
        self.by_height.keys().next_back().copied()
    }

    /// Get the number of entries in the index.
    pub fn len(&self) -> usize {
        self.by_height.len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.by_height.is_empty()
    }

    /// Iterate over all entries in height order.
    pub fn iter(&self) -> impl Iterator<Item = &IndexEntry> {
        self.by_height.values()
    }

    /// Get entries in a height range (inclusive).
    pub fn range(&self, start: u64, end: u64) -> impl Iterator<Item = &IndexEntry> {
        self.by_height.range(start..=end).map(|(_, v)| v)
    }
}

/// Serde helper for hex-encoded bytes.
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_index_operations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-index.jsonl");

        // Create and populate index
        {
            let mut index = BlockIndex::open(&path).await.unwrap();
            assert!(index.is_empty());

            let entry = IndexEntry {
                block_height: 1,
                block_hash: B256::from([1u8; 32]),
                celestia_height: 100,
                commitment: vec![1, 2, 3],
            };
            index.insert(entry).await.unwrap();

            assert_eq!(index.len(), 1);
            assert_eq!(index.latest_height(), Some(1));
            assert!(index.get_by_height(1).is_some());
            assert!(index.get_by_hash(&B256::from([1u8; 32])).is_some());
        }

        // Reopen and verify persistence
        {
            let index = BlockIndex::open(&path).await.unwrap();
            assert_eq!(index.len(), 1);
            assert_eq!(index.latest_height(), Some(1));

            let entry = index.get_by_height(1).unwrap();
            assert_eq!(entry.celestia_height, 100);
        }
    }
}
