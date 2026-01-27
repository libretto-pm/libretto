//! Checksum verification using multiple hash algorithms.
//!
//! Supports SHA-256, SHA-1, and SIMD-accelerated BLAKE3.

use crate::config::{ChecksumType, ExpectedChecksum};
use crate::error::{DownloadError, Result};
use blake3::Hasher as Blake3Hasher;
use digest::Digest;
use sha1::Sha1;
use sha2::Sha256;
use std::io::Read;
use std::path::Path;

/// Multi-algorithm incremental hasher.
///
/// Computes checksums incrementally as data is streamed,
/// supporting multiple hash algorithms simultaneously.
#[derive(Debug)]
pub struct MultiHasher {
    blake3: Blake3Hasher,
    sha256: Option<Sha256>,
    sha1: Option<Sha1>,
}

impl Default for MultiHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiHasher {
    /// Create a new hasher that computes BLAKE3 by default.
    #[must_use]
    pub fn new() -> Self {
        Self {
            blake3: Blake3Hasher::new(),
            sha256: None,
            sha1: None,
        }
    }

    /// Create a hasher configured to compute specific checksums.
    #[must_use]
    pub fn for_checksums(expected: &[ExpectedChecksum]) -> Self {
        let mut hasher = Self::new();
        for checksum in expected {
            match checksum.checksum_type {
                ChecksumType::Sha256 => hasher.sha256 = Some(Sha256::new()),
                ChecksumType::Sha1 => hasher.sha1 = Some(Sha1::new()),
                ChecksumType::Blake3 => {} // Always enabled
            }
        }
        hasher
    }

    /// Create a hasher that computes all supported algorithms.
    #[must_use]
    pub fn all() -> Self {
        Self {
            blake3: Blake3Hasher::new(),
            sha256: Some(Sha256::new()),
            sha1: Some(Sha1::new()),
        }
    }

    /// Update the hasher with data.
    pub fn update(&mut self, data: &[u8]) {
        self.blake3.update(data);
        if let Some(ref mut h) = self.sha256 {
            h.update(data);
        }
        if let Some(ref mut h) = self.sha1 {
            h.update(data);
        }
    }

    /// Finalize and get BLAKE3 hash.
    #[must_use]
    pub fn finalize_blake3(self) -> [u8; 32] {
        *self.blake3.finalize().as_bytes()
    }

    /// Finalize and get all computed hashes.
    #[must_use]
    pub fn finalize(self) -> ComputedChecksums {
        let blake3 = *self.blake3.finalize().as_bytes();
        let sha256 = self.sha256.map(|h| {
            let result = h.finalize();
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&result);
            bytes
        });
        let sha1 = self.sha1.map(|h| {
            let result = h.finalize();
            let mut bytes = [0u8; 20];
            bytes.copy_from_slice(&result);
            bytes
        });

        ComputedChecksums {
            blake3,
            sha256,
            sha1,
        }
    }
}

/// Computed checksums from finalized hasher.
#[derive(Debug, Clone)]
pub struct ComputedChecksums {
    /// BLAKE3 hash (always computed).
    pub blake3: [u8; 32],
    /// SHA-256 hash (if requested).
    pub sha256: Option<[u8; 32]>,
    /// SHA-1 hash (if requested).
    pub sha1: Option<[u8; 20]>,
}

impl ComputedChecksums {
    /// Get BLAKE3 as hex string.
    #[must_use]
    pub fn blake3_hex(&self) -> String {
        bytes_to_hex(&self.blake3)
    }

    /// Get SHA-256 as hex string if computed.
    #[must_use]
    pub fn sha256_hex(&self) -> Option<String> {
        self.sha256.as_ref().map(|h| bytes_to_hex(h))
    }

    /// Get SHA-1 as hex string if computed.
    #[must_use]
    pub fn sha1_hex(&self) -> Option<String> {
        self.sha1.as_ref().map(|h| bytes_to_hex(h))
    }

    /// Get hex string for a specific checksum type.
    #[must_use]
    pub fn get_hex(&self, checksum_type: &ChecksumType) -> Option<String> {
        match checksum_type {
            ChecksumType::Blake3 => Some(self.blake3_hex()),
            ChecksumType::Sha256 => self.sha256_hex(),
            ChecksumType::Sha1 => self.sha1_hex(),
        }
    }

    /// Verify against expected checksums.
    ///
    /// # Errors
    /// Returns `ChecksumMismatch` error if any checksum doesn't match.
    pub fn verify(&self, expected: &[ExpectedChecksum], name: &str) -> Result<()> {
        for exp in expected {
            let actual = self.get_hex(&exp.checksum_type).ok_or_else(|| {
                DownloadError::ChecksumMismatch {
                    name: name.to_string(),
                    expected: exp.value.clone(),
                    actual: format!("{:?} not computed", exp.checksum_type),
                }
            })?;

            if !constant_time_compare(&actual, &exp.value) {
                return Err(DownloadError::ChecksumMismatch {
                    name: name.to_string(),
                    expected: exp.value.clone(),
                    actual,
                });
            }
        }
        Ok(())
    }
}

/// Convert bytes to lowercase hex string.
#[must_use]
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0x0f) as usize] as char);
    }
    s
}

/// Parse hex string to bytes.
#[must_use]
pub fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks(2) {
        let s = std::str::from_utf8(chunk).ok()?;
        let byte = u8::from_str_radix(s, 16).ok()?;
        bytes.push(byte);
    }
    Some(bytes)
}

/// Constant-time string comparison to prevent timing attacks.
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        result |= x ^ y;
    }
    result == 0
}

/// Compute BLAKE3 hash of a file.
///
/// Uses SIMD acceleration when available.
///
/// # Errors
/// Returns I/O error if file cannot be read.
pub fn blake3_file(path: &Path) -> Result<[u8; 32]> {
    let file = std::fs::File::open(path).map_err(|e| DownloadError::io(path, e))?;
    let mut reader = std::io::BufReader::with_capacity(128 * 1024, file);
    let mut hasher = Blake3Hasher::new();
    let mut buf = vec![0u8; 128 * 1024];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| DownloadError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(*hasher.finalize().as_bytes())
}

/// Compute SHA-256 hash of a file.
///
/// # Errors
/// Returns I/O error if file cannot be read.
pub fn sha256_file(path: &Path) -> Result<[u8; 32]> {
    let file = std::fs::File::open(path).map_err(|e| DownloadError::io(path, e))?;
    let mut reader = std::io::BufReader::with_capacity(128 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 128 * 1024];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| DownloadError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let result = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&result);
    Ok(bytes)
}

/// Compute SHA-1 hash of a file.
///
/// # Errors
/// Returns I/O error if file cannot be read.
pub fn sha1_file(path: &Path) -> Result<[u8; 20]> {
    let file = std::fs::File::open(path).map_err(|e| DownloadError::io(path, e))?;
    let mut reader = std::io::BufReader::with_capacity(128 * 1024, file);
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 128 * 1024];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| DownloadError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let result = hasher.finalize();
    let mut bytes = [0u8; 20];
    bytes.copy_from_slice(&result);
    Ok(bytes)
}

/// Verify file against expected checksum.
///
/// # Errors
/// Returns error if file cannot be read or checksum doesn't match.
pub fn verify_file(path: &Path, expected: &ExpectedChecksum, name: &str) -> Result<()> {
    let actual_hex = match expected.checksum_type {
        ChecksumType::Blake3 => bytes_to_hex(&blake3_file(path)?),
        ChecksumType::Sha256 => bytes_to_hex(&sha256_file(path)?),
        ChecksumType::Sha1 => bytes_to_hex(&sha1_file(path)?),
    };

    if !constant_time_compare(&actual_hex, &expected.value) {
        return Err(DownloadError::ChecksumMismatch {
            name: name.to_string(),
            expected: expected.value.clone(),
            actual: actual_hex,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_hasher_basic() {
        let mut hasher = MultiHasher::new();
        hasher.update(b"hello world");
        let checksums = hasher.finalize();

        // BLAKE3 is always computed
        assert_eq!(checksums.blake3_hex().len(), 64);
        // SHA256/SHA1 not computed by default
        assert!(checksums.sha256.is_none());
        assert!(checksums.sha1.is_none());
    }

    #[test]
    fn multi_hasher_all() {
        let mut hasher = MultiHasher::all();
        hasher.update(b"test");
        let checksums = hasher.finalize();

        assert!(checksums.sha256.is_some());
        assert!(checksums.sha1.is_some());

        // Known SHA-256 of "test"
        assert_eq!(
            checksums.sha256_hex().unwrap(),
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }

    #[test]
    fn hex_roundtrip() {
        let bytes = [0x12, 0x34, 0xab, 0xcd];
        let hex = bytes_to_hex(&bytes);
        assert_eq!(hex, "1234abcd");
        let recovered = hex_to_bytes(&hex).unwrap();
        assert_eq!(recovered, bytes);
    }

    #[test]
    fn verify_checksum() {
        let mut hasher = MultiHasher::all();
        hasher.update(b"test");
        let checksums = hasher.finalize();

        let expected = vec![ExpectedChecksum::sha256(
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        )];

        assert!(checksums.verify(&expected, "test").is_ok());

        let wrong = vec![ExpectedChecksum::sha256("0".repeat(64))];
        assert!(checksums.verify(&wrong, "test").is_err());
    }
}
