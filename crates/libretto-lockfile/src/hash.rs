//! SIMD-accelerated content hashing and integrity verification.
//!
//! Provides ultra-high-performance hashing using:
//! - MD5 for Composer content-hash compatibility
//! - BLAKE3 for fast integrity verification (SIMD-accelerated)
//! - Parallel hashing for large files

use blake3::Hasher as Blake3Hasher;
use digest::Digest;
use md5::Md5;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::io::Read;

/// Content hash for composer.json dependencies.
///
/// Uses MD5 to match Composer's content-hash format.
#[derive(Debug)]
pub struct ContentHasher {
    hasher: Md5,
}

impl Default for ContentHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentHasher {
    /// Create a new content hasher.
    #[must_use]
    pub fn new() -> Self {
        Self { hasher: Md5::new() }
    }

    /// Update hasher with data.
    pub fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Finalize and return hex string.
    #[must_use]
    pub fn finalize(self) -> String {
        let result = self.hasher.finalize();
        bytes_to_hex(&result)
    }

    /// Compute content-hash from composer.json dependencies.
    ///
    /// Matches Composer's exact algorithm:
    /// - Sort require/require-dev alphabetically
    /// - Serialize to JSON without whitespace
    /// - MD5 hash the result
    #[must_use]
    pub fn compute_content_hash(
        require: &BTreeMap<String, String>,
        require_dev: &BTreeMap<String, String>,
        minimum_stability: Option<&str>,
        prefer_stable: Option<bool>,
        prefer_lowest: Option<bool>,
        platform: &BTreeMap<String, String>,
        platform_overrides: &BTreeMap<String, String>,
    ) -> String {
        let mut hasher = Self::new();

        // Build deterministic JSON representation
        // Composer uses specific field ordering and formatting
        let mut parts = Vec::with_capacity(7);

        // require (already sorted by BTreeMap)
        if !require.is_empty() {
            parts.push(format!("\"require\":{}", btree_to_json(require)));
        }

        // require-dev
        if !require_dev.is_empty() {
            parts.push(format!("\"require-dev\":{}", btree_to_json(require_dev)));
        }

        // minimum-stability
        if let Some(stability) = minimum_stability {
            parts.push(format!("\"minimum-stability\":\"{}\"", stability));
        }

        // prefer-stable
        if let Some(prefer) = prefer_stable {
            parts.push(format!("\"prefer-stable\":{}", prefer));
        }

        // prefer-lowest
        if let Some(prefer) = prefer_lowest {
            parts.push(format!("\"prefer-lowest\":{}", prefer));
        }

        // platform
        if !platform.is_empty() {
            parts.push(format!("\"platform\":{}", btree_to_json(platform)));
        }

        // platform-overrides (config.platform)
        if !platform_overrides.is_empty() {
            parts.push(format!(
                "\"platform-overrides\":{}",
                btree_to_json(platform_overrides)
            ));
        }

        let json = format!("{{{}}}", parts.join(","));
        hasher.update(json.as_bytes());
        hasher.finalize()
    }
}

/// Convert BTreeMap to minimal JSON.
fn btree_to_json(map: &BTreeMap<String, String>) -> String {
    let pairs: Vec<String> = map
        .iter()
        .map(|(k, v)| {
            format!(
                "\"{}\":\"{}\"",
                escape_json_string(k),
                escape_json_string(v)
            )
        })
        .collect();
    format!("{{{}}}", pairs.join(","))
}

/// Escape special characters in JSON strings.
fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

/// SIMD-accelerated BLAKE3 hasher for integrity verification.
#[derive(Debug)]
pub struct IntegrityHasher {
    hasher: Blake3Hasher,
}

impl Default for IntegrityHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl IntegrityHasher {
    /// Create a new integrity hasher.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hasher: Blake3Hasher::new(),
        }
    }

    /// Update with data.
    pub fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Finalize to 32-byte hash.
    #[must_use]
    pub fn finalize(self) -> [u8; 32] {
        *self.hasher.finalize().as_bytes()
    }

    /// Finalize to hex string.
    #[must_use]
    pub fn finalize_hex(self) -> String {
        bytes_to_hex(self.hasher.finalize().as_bytes())
    }

    /// Hash bytes directly.
    #[must_use]
    pub fn hash_bytes(data: &[u8]) -> [u8; 32] {
        *blake3::hash(data).as_bytes()
    }

    /// Hash bytes to hex string.
    #[must_use]
    pub fn hash_bytes_hex(data: &[u8]) -> String {
        bytes_to_hex(blake3::hash(data).as_bytes())
    }

    /// Hash a file using memory-mapped I/O for large files.
    ///
    /// # Errors
    /// Returns I/O error if file cannot be read.
    ///
    /// # Safety
    /// Uses memory-mapped I/O for large files, which is safe as long as the file
    /// is not modified while being read (standard file locking applies).
    #[allow(unsafe_code)]
    pub fn hash_file(path: &std::path::Path) -> std::io::Result<[u8; 32]> {
        let file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        let len = metadata.len();

        // Use memory mapping for large files (>1MB)
        // SAFETY: The file is opened read-only and we don't modify it
        if len > 1024 * 1024 {
            let mmap = unsafe { memmap2::Mmap::map(&file)? };
            Ok(Self::hash_bytes(&mmap))
        } else {
            // Small files: buffered read
            let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
            let mut hasher = Self::new();
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.finalize())
        }
    }
}

/// Parallel hash computation for multiple files.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParallelHasher;

impl ParallelHasher {
    /// Hash multiple files in parallel.
    ///
    /// Returns a map of path -> hash for successful hashes.
    #[must_use]
    pub fn hash_files(paths: &[std::path::PathBuf]) -> Vec<(std::path::PathBuf, Option<[u8; 32]>)> {
        paths
            .par_iter()
            .map(|path| {
                let hash = IntegrityHasher::hash_file(path).ok();
                (path.clone(), hash)
            })
            .collect()
    }

    /// Hash multiple byte slices in parallel.
    #[must_use]
    pub fn hash_slices(slices: &[&[u8]]) -> Vec<[u8; 32]> {
        slices
            .par_iter()
            .map(|data| IntegrityHasher::hash_bytes(data))
            .collect()
    }
}

/// SIMD-accelerated hash comparison.
///
/// # Safety
/// Uses AVX2 SIMD intrinsics for fast comparison.
#[inline]
#[allow(unsafe_code)]
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
pub fn compare_hashes_simd(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use std::arch::x86_64::{__m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8};
    // SAFETY: AVX2 is available (checked by cfg), pointers are valid 32-byte aligned arrays
    unsafe {
        let va = _mm256_loadu_si256(a.as_ptr() as *const __m256i);
        let vb = _mm256_loadu_si256(b.as_ptr() as *const __m256i);
        let cmp = _mm256_cmpeq_epi8(va, vb);
        let mask = _mm256_movemask_epi8(cmp);
        mask == -1i32
    }
}

/// Fallback hash comparison.
#[inline]
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
pub fn compare_hashes_simd(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a == b
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

/// Constant-time comparison to prevent timing attacks.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// Verify a hash matches expected value (constant-time).
#[must_use]
pub fn verify_hash(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    constant_time_eq(actual.as_bytes(), expected.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_empty() {
        let hash = ContentHasher::compute_content_hash(
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            None,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        // MD5 of "{}"
        assert_eq!(hash, "99914b932bd37a50b983c5e7c90ae93b");
    }

    #[test]
    fn test_content_hash_with_deps() {
        let mut require = BTreeMap::new();
        require.insert("psr/log".to_string(), "^3.0".to_string());
        require.insert("symfony/console".to_string(), "^6.0".to_string());

        let hash = ContentHasher::compute_content_hash(
            &require,
            &BTreeMap::new(),
            Some("stable"),
            Some(true),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
        );

        // Should be deterministic
        let hash2 = ContentHasher::compute_content_hash(
            &require,
            &BTreeMap::new(),
            Some("stable"),
            Some(true),
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_integrity_hash() {
        let data = b"hello world";
        let hash = IntegrityHasher::hash_bytes(data);
        let hash2 = IntegrityHasher::hash_bytes(data);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_hex_roundtrip() {
        let bytes = [0x12u8, 0x34, 0xab, 0xcd, 0xef];
        let hex = bytes_to_hex(&bytes);
        assert_eq!(hex, "1234abcdef");
        let recovered = hex_to_bytes(&hex).unwrap();
        assert_eq!(recovered, bytes);
    }

    #[test]
    fn test_hash_comparison() {
        let a = [0u8; 32];
        let b = [0u8; 32];
        let c = [1u8; 32];

        assert!(compare_hashes_simd(&a, &b));
        assert!(!compare_hashes_simd(&a, &c));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
    }

    #[test]
    fn test_verify_hash() {
        assert!(verify_hash("abc123", "abc123"));
        assert!(!verify_hash("abc123", "abc124"));
        assert!(!verify_hash("abc123", "abc12"));
    }

    #[test]
    fn test_escape_json() {
        assert_eq!(escape_json_string("hello"), "hello");
        assert_eq!(escape_json_string("hello\"world"), "hello\\\"world");
        assert_eq!(escape_json_string("a\\b"), "a\\\\b");
        assert_eq!(escape_json_string("a\nb"), "a\\nb");
    }
}
