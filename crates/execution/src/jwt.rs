//! JWT token generation for Engine API authentication.
//!
//! Implements HS256 (HMAC-SHA256) JWT tokens as required by the
//! Ethereum Engine API specification.

use sha2::{Digest, Sha256};

/// HMAC-SHA256 block size in bytes.
const HMAC_BLOCK_SIZE: usize = 64;

/// Build a JWT token for Engine API authentication.
///
/// # Arguments
///
/// * `secret_hex` - The 32-byte secret as a hex string (with or without 0x prefix)
///
/// # Returns
///
/// A JWT token string in the format `header.payload.signature`.
///
/// # Errors
///
/// Returns an error if the secret is invalid (not valid hex or wrong length).
pub fn build_token(secret_hex: &str) -> Result<String, String> {
    use alloy_primitives::hex;

    // Decode hex secret
    let secret_bytes = hex::decode(secret_hex.trim_start_matches("0x"))
        .map_err(|e| format!("invalid hex: {e}"))?;

    if secret_bytes.len() != 32 {
        return Err(format!(
            "JWT secret must be 32 bytes, got {}",
            secret_bytes.len()
        ));
    }

    // Build JWT header and payload
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let payload = format!(r#"{{"iat":{now}}}"#);

    // Base64url encode
    let header_b64 = base64url_encode(header.as_bytes());
    let payload_b64 = base64url_encode(payload.as_bytes());

    let message = format!("{header_b64}.{payload_b64}");

    // HMAC-SHA256 signature
    let signature = hmac_sha256(&secret_bytes, message.as_bytes());
    let signature_b64 = base64url_encode(&signature);

    Ok(format!("{message}.{signature_b64}"))
}

/// Base64url encode without padding.
fn base64url_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        let b0 = data[i] as usize;
        let b1 = data.get(i + 1).copied().unwrap_or(0) as usize;
        let b2 = data.get(i + 2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if i + 1 < data.len() {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        }
        if i + 2 < data.len() {
            result.push(ALPHABET[b2 & 0x3f] as char);
        }

        i += 3;
    }

    result
}

/// HMAC-SHA256 implementation.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    // Prepare key
    let mut key_block = [0u8; HMAC_BLOCK_SIZE];
    if key.len() > HMAC_BLOCK_SIZE {
        let hash = Sha256::digest(key);
        key_block[..32].copy_from_slice(&hash);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    // Inner padding
    let mut inner = [0x36u8; HMAC_BLOCK_SIZE];
    for (i, b) in key_block.iter().enumerate() {
        inner[i] ^= b;
    }

    // Outer padding
    let mut outer = [0x5cu8; HMAC_BLOCK_SIZE];
    for (i, b) in key_block.iter().enumerate() {
        outer[i] ^= b;
    }

    // Inner hash
    let mut hasher = Sha256::new();
    hasher.update(inner);
    hasher.update(message);
    let inner_hash = hasher.finalize();

    // Outer hash
    let mut hasher = Sha256::new();
    hasher.update(outer);
    hasher.update(inner_hash);
    let result = hasher.finalize();

    let mut output = [0u8; 32];
    output.copy_from_slice(&result);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_token() {
        let secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let token = build_token(secret);
        assert!(token.is_ok());
        let token = token.unwrap();
        // JWT should have 3 parts separated by dots
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn test_build_token_with_0x_prefix() {
        let secret = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let token = build_token(secret);
        assert!(token.is_ok());
    }

    #[test]
    fn test_build_token_invalid_length() {
        let secret = "0123456789abcdef"; // Too short
        let result = build_token(secret);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be 32 bytes"));
    }

    #[test]
    fn test_build_token_invalid_hex() {
        let secret = "not_valid_hex_0123456789abcdef0123456789abcdef0123456789";
        let result = build_token(secret);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid hex"));
    }

    #[test]
    fn test_base64url_encode() {
        assert_eq!(base64url_encode(b"hello"), "aGVsbG8");
        assert_eq!(base64url_encode(b""), "");
    }
}
