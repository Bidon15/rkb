//! Serde helpers for common patterns.
//!
//! This module provides reusable serde serialization/deserialization helpers
//! for hex-encoded byte arrays.

/// Hex-encoded fixed-size byte arrays.
///
/// Use with `#[serde(with = "sequencer_types::serde_helpers::hex_fixed")]`
/// for `[u8; N]` fields.
pub mod hex_fixed {
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize a fixed-size byte array as a hex string.
    pub fn serialize<S, const N: usize>(bytes: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    /// Deserialize a hex string into a fixed-size byte array.
    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<[u8; N], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        let len = bytes.len();
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom(format!("expected {N} bytes, got {len}")))
    }
}

/// Hex-encoded variable-length bytes.
///
/// Use with `#[serde(with = "sequencer_types::serde_helpers::hex_vec")]`
/// for `Vec<u8>` fields.
pub mod hex_vec {
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize a byte slice as a hex string.
    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    /// Deserialize a hex string into a `Vec<u8>`.
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
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct FixedBytes {
        #[serde(with = "hex_fixed")]
        data: [u8; 4],
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct VecBytes {
        #[serde(with = "hex_vec")]
        data: Vec<u8>,
    }

    #[test]
    fn test_hex_fixed_roundtrip() {
        let original = FixedBytes {
            data: [0xde, 0xad, 0xbe, 0xef],
        };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, r#"{"data":"deadbeef"}"#);

        let decoded: FixedBytes = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_hex_fixed_wrong_length() {
        let json = r#"{"data":"deadbe"}"#; // 3 bytes, not 4
        let result: Result<FixedBytes, _> = serde_json::from_str(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expected 4 bytes"));
    }

    #[test]
    fn test_hex_vec_roundtrip() {
        let original = VecBytes {
            data: vec![0xca, 0xfe, 0xba, 0xbe],
        };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, r#"{"data":"cafebabe"}"#);

        let decoded: VecBytes = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_hex_vec_empty() {
        let original = VecBytes { data: vec![] };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, r#"{"data":""}"#);

        let decoded: VecBytes = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_hex_invalid() {
        let json = r#"{"data":"not_hex"}"#;
        let result: Result<VecBytes, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
