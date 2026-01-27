//! Zstd compression utilities for cache entries.

use std::io::{Read, Write};

/// Compress data using zstd.
///
/// # Errors
/// Returns error if compression fails.
pub fn compress(data: &[u8], level: i32) -> std::io::Result<Vec<u8>> {
    let mut encoder = zstd::Encoder::new(Vec::new(), level)?;
    encoder.write_all(data)?;
    encoder.finish()
}

/// Decompress zstd data.
///
/// # Errors
/// Returns error if decompression fails.
pub fn decompress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut decoder = zstd::Decoder::new(data)?;
    let mut output = Vec::new();
    decoder.read_to_end(&mut output)?;
    Ok(output)
}

/// Decompress with expected size hint.
///
/// # Errors
/// Returns error if decompression fails.
pub fn decompress_with_hint(data: &[u8], size_hint: usize) -> std::io::Result<Vec<u8>> {
    let mut decoder = zstd::Decoder::new(data)?;
    let mut output = Vec::with_capacity(size_hint);
    decoder.read_to_end(&mut output)?;
    Ok(output)
}

/// Check if data is worth compressing based on size.
/// Very small data may not benefit from compression.
#[must_use]
pub fn should_compress(data: &[u8]) -> bool {
    // Don't compress data smaller than 100 bytes
    data.len() >= 100
}

/// Compression statistics.
#[derive(Debug, Clone, Copy)]
pub struct CompressionStats {
    /// Original size in bytes.
    pub original_size: usize,
    /// Compressed size in bytes.
    pub compressed_size: usize,
}

impl CompressionStats {
    /// Calculate compression ratio (compressed/original).
    #[must_use]
    pub fn ratio(&self) -> f64 {
        if self.original_size == 0 {
            1.0
        } else {
            self.compressed_size as f64 / self.original_size as f64
        }
    }

    /// Calculate space savings percentage.
    #[must_use]
    pub fn savings_percent(&self) -> f64 {
        (1.0 - self.ratio()) * 100.0
    }

    /// Bytes saved by compression.
    #[must_use]
    pub fn bytes_saved(&self) -> usize {
        self.original_size.saturating_sub(self.compressed_size)
    }
}

/// Compress and return stats.
///
/// # Errors
/// Returns error if compression fails.
pub fn compress_with_stats(
    data: &[u8],
    level: i32,
) -> std::io::Result<(Vec<u8>, CompressionStats)> {
    let compressed = compress(data, level)?;
    let stats = CompressionStats {
        original_size: data.len(),
        compressed_size: compressed.len(),
    };
    Ok((compressed, stats))
}

/// Magic bytes to identify compressed cache entries.
pub const COMPRESSED_MAGIC: &[u8; 4] = b"ZSTD";

/// Prepend magic bytes to compressed data.
#[must_use]
pub fn with_magic(data: Vec<u8>) -> Vec<u8> {
    let mut result = Vec::with_capacity(COMPRESSED_MAGIC.len() + data.len());
    result.extend_from_slice(COMPRESSED_MAGIC);
    result.extend(data);
    result
}

/// Check if data has compression magic and strip it.
#[must_use]
pub fn strip_magic(data: &[u8]) -> Option<&[u8]> {
    if data.starts_with(COMPRESSED_MAGIC) {
        Some(&data[COMPRESSED_MAGIC.len()..])
    } else {
        None
    }
}

/// Check if data appears to be compressed.
#[must_use]
pub fn is_compressed(data: &[u8]) -> bool {
    data.starts_with(COMPRESSED_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_decompress() {
        let original = b"Hello, World! This is some test data that should compress well. \
                        Repeating content helps compression: aaaaaaaaaaaabbbbbbbbbbbb";

        let compressed = compress(original, 3).expect("compression should work");
        assert!(compressed.len() < original.len());

        let decompressed = decompress(&compressed).expect("decompression should work");
        assert_eq!(decompressed, original);
    }

    #[test]
    fn compression_stats() {
        let data = vec![0u8; 1000]; // Highly compressible

        let (_compressed, stats) = compress_with_stats(&data, 3).expect("should compress");

        assert!(stats.ratio() < 0.5);
        assert!(stats.savings_percent() > 50.0);
        assert!(stats.bytes_saved() > 500);
    }

    #[test]
    fn magic_bytes() {
        let data = vec![1, 2, 3, 4, 5];
        let with_magic = with_magic(data.clone());

        assert!(is_compressed(&with_magic));
        assert!(!is_compressed(&data));

        let stripped = strip_magic(&with_magic).expect("should have magic");
        assert_eq!(stripped, &data[..]);
    }

    #[test]
    fn should_compress_threshold() {
        assert!(!should_compress(&[0u8; 50]));
        assert!(should_compress(&[0u8; 100]));
        assert!(should_compress(&[0u8; 1000]));
    }
}
