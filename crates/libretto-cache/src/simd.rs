//! SIMD-accelerated operations for cache key comparison and searching.
//!
//! This module provides high-performance operations using SIMD intrinsics
//! where available, with fallback implementations for unsupported platforms.

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
use std::arch::x86_64::{
    __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8,
};

/// SIMD-accelerated byte comparison for cache keys (32-byte BLAKE3 hashes).
///
/// Returns true if the two 32-byte slices are equal.
#[inline]
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[must_use]
pub fn compare_hash_simd(a: &[u8; 32], b: &[u8; 32]) -> bool {
    unsafe {
        let va = _mm256_loadu_si256(a.as_ptr().cast::<__m256i>());
        let vb = _mm256_loadu_si256(b.as_ptr().cast::<__m256i>());
        let cmp = _mm256_cmpeq_epi8(va, vb);
        let mask = _mm256_movemask_epi8(cmp);
        mask == -1i32 // All 32 bytes equal
    }
}

/// Fallback comparison for non-AVX2 platforms.
#[inline]
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
pub fn compare_hash_simd(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a == b
}

/// SIMD-accelerated search for a byte pattern in a larger buffer.
/// Returns the index of the first occurrence, or None if not found.
#[inline]
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[must_use]
pub fn find_byte_simd(haystack: &[u8], needle: u8) -> Option<usize> {
    if haystack.is_empty() {
        return None;
    }

    unsafe {
        let needle_vec = _mm256_set1_epi8(needle as i8);
        let chunks = haystack.len() / 32;

        for i in 0..chunks {
            let offset = i * 32;
            let chunk = _mm256_loadu_si256(haystack.as_ptr().add(offset).cast::<__m256i>());
            let cmp = _mm256_cmpeq_epi8(chunk, needle_vec);
            let mask = _mm256_movemask_epi8(cmp) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
        }

        // Handle remaining bytes
        let remaining_start = chunks * 32;
        for (i, &byte) in haystack[remaining_start..].iter().enumerate() {
            if byte == needle {
                return Some(remaining_start + i);
            }
        }
    }

    None
}

/// Fallback byte search for non-AVX2 platforms.
#[inline]
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
pub fn find_byte_simd(haystack: &[u8], needle: u8) -> Option<usize> {
    memchr::memchr(needle, haystack)
}

/// SIMD-accelerated prefix matching for namespace lookups.
/// Checks if `haystack` starts with `prefix`.
#[inline]
#[must_use]
pub fn starts_with_simd(haystack: &[u8], prefix: &[u8]) -> bool {
    if prefix.len() > haystack.len() {
        return false;
    }

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        if prefix.len() >= 32 {
            return starts_with_simd_avx2(haystack, prefix);
        }
    }

    // Fallback for short prefixes or non-AVX2
    haystack.starts_with(prefix)
}

/// AVX2 implementation of prefix matching.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline]
fn starts_with_simd_avx2(haystack: &[u8], prefix: &[u8]) -> bool {
    unsafe {
        let chunks = prefix.len() / 32;

        for i in 0..chunks {
            let offset = i * 32;
            let h = _mm256_loadu_si256(haystack.as_ptr().add(offset).cast::<__m256i>());
            let p = _mm256_loadu_si256(prefix.as_ptr().add(offset).cast::<__m256i>());
            let cmp = _mm256_cmpeq_epi8(h, p);
            let mask = _mm256_movemask_epi8(cmp);
            if mask != -1i32 {
                return false;
            }
        }

        // Check remaining bytes
        let remaining_start = chunks * 32;
        haystack[remaining_start..].starts_with(&prefix[remaining_start..])
    }
}

/// Batch compare multiple hashes against a target.
/// Returns indices of matching hashes.
#[inline]
#[must_use]
pub fn find_matching_hashes(hashes: &[[u8; 32]], target: &[u8; 32]) -> Vec<usize> {
    hashes
        .iter()
        .enumerate()
        .filter(|(_, h)| compare_hash_simd(h, target))
        .map(|(i, _)| i)
        .collect()
}

/// SIMD-accelerated counting of set bits (popcount) for bloom filter.
#[inline]
#[cfg(all(target_arch = "x86_64", target_feature = "popcnt"))]
#[must_use]
pub const fn popcount_u64(x: u64) -> u32 {
    x.count_ones()
}

/// Fallback popcount.
#[inline]
#[cfg(not(all(target_arch = "x86_64", target_feature = "popcnt")))]
pub fn popcount_u64(x: u64) -> u32 {
    x.count_ones()
}

/// Batch popcount for bloom filter bit arrays.
#[inline]
#[must_use]
pub fn popcount_slice(data: &[u64]) -> usize {
    data.iter().map(|&x| popcount_u64(x) as usize).sum()
}

/// SIMD-accelerated XOR for bloom filter operations.
#[inline]
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
pub fn xor_slices_simd(a: &mut [u64], b: &[u64]) {
    use std::arch::x86_64::{_mm256_loadu_si256, _mm256_storeu_si256, _mm256_xor_si256};

    assert_eq!(a.len(), b.len());

    let chunks = a.len() / 4; // 4 u64s = 256 bits

    unsafe {
        for i in 0..chunks {
            let offset = i * 4;
            let va = _mm256_loadu_si256(a.as_ptr().add(offset).cast::<__m256i>());
            let vb = _mm256_loadu_si256(b.as_ptr().add(offset).cast::<__m256i>());
            let result = _mm256_xor_si256(va, vb);
            _mm256_storeu_si256(a.as_mut_ptr().add(offset).cast::<__m256i>(), result);
        }

        // Handle remaining elements
        let remaining_start = chunks * 4;
        for i in remaining_start..a.len() {
            a[i] ^= b[i];
        }
    }
}

/// Fallback XOR for non-AVX2 platforms.
#[inline]
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
pub fn xor_slices_simd(a: &mut [u64], b: &[u64]) {
    assert_eq!(a.len(), b.len());
    for (x, &y) in a.iter_mut().zip(b.iter()) {
        *x ^= y;
    }
}

/// SIMD-accelerated OR for bloom filter union.
#[inline]
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
pub fn or_slices_simd(a: &mut [u64], b: &[u64]) {
    use std::arch::x86_64::{_mm256_loadu_si256, _mm256_or_si256, _mm256_storeu_si256};

    assert_eq!(a.len(), b.len());

    let chunks = a.len() / 4;

    unsafe {
        for i in 0..chunks {
            let offset = i * 4;
            let va = _mm256_loadu_si256(a.as_ptr().add(offset).cast::<__m256i>());
            let vb = _mm256_loadu_si256(b.as_ptr().add(offset).cast::<__m256i>());
            let result = _mm256_or_si256(va, vb);
            _mm256_storeu_si256(a.as_mut_ptr().add(offset).cast::<__m256i>(), result);
        }

        let remaining_start = chunks * 4;
        for i in remaining_start..a.len() {
            a[i] |= b[i];
        }
    }
}

/// Fallback OR for non-AVX2 platforms.
#[inline]
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
pub fn or_slices_simd(a: &mut [u64], b: &[u64]) {
    assert_eq!(a.len(), b.len());
    for (x, &y) in a.iter_mut().zip(b.iter()) {
        *x |= y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_comparison() {
        let a = [0u8; 32];
        let b = [0u8; 32];
        let c = [1u8; 32];

        assert!(compare_hash_simd(&a, &b));
        assert!(!compare_hash_simd(&a, &c));
    }

    #[test]
    fn test_find_byte() {
        let data = b"hello world";
        assert_eq!(find_byte_simd(data, b'w'), Some(6));
        assert_eq!(find_byte_simd(data, b'x'), None);
        assert_eq!(find_byte_simd(data, b'h'), Some(0));
    }

    #[test]
    fn test_starts_with() {
        let haystack = b"hello world this is a test";
        assert!(starts_with_simd(haystack, b"hello"));
        assert!(starts_with_simd(haystack, b"hello world"));
        assert!(!starts_with_simd(haystack, b"world"));
    }

    #[test]
    fn test_popcount() {
        assert_eq!(popcount_u64(0), 0);
        assert_eq!(popcount_u64(1), 1);
        assert_eq!(popcount_u64(0xFF), 8);
        assert_eq!(popcount_u64(u64::MAX), 64);
    }

    #[test]
    fn test_popcount_slice() {
        let data = [0u64, 1, 0xFF, u64::MAX];
        assert_eq!(popcount_slice(&data), 73); // 0 + 1 + 8 + 64
    }

    #[test]
    fn test_xor_slices() {
        let mut a = [0xFFu64, 0, 0xAA, 0x55];
        let b = [0xFFu64, 0xFF, 0x55, 0xAA];
        xor_slices_simd(&mut a, &b);
        assert_eq!(a, [0, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn test_or_slices() {
        let mut a = [0xF0u64, 0, 0xAA, 0];
        let b = [0x0Fu64, 0xFF, 0x55, 0xAA];
        or_slices_simd(&mut a, &b);
        assert_eq!(a, [0xFF, 0xFF, 0xFF, 0xAA]);
    }

    #[test]
    fn test_find_matching_hashes() {
        let target = [42u8; 32];
        let hashes = vec![[0u8; 32], [42u8; 32], [1u8; 32], [42u8; 32]];
        let matches = find_matching_hashes(&hashes, &target);
        assert_eq!(matches, vec![1, 3]);
    }
}
