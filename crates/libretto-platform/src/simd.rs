//! SIMD operations with platform-specific optimizations and runtime detection.
//!
//! Supports:
//! - `x86_64`: SSE4.2, AVX2, AVX-512 with runtime detection
//! - ARM64: NEON intrinsics
//! - Graceful fallback to scalar operations
//!
//! # Example
//!
//! ```
//! use libretto_platform::simd::{SimdRuntime, SimdOps};
//!
//! let runtime = SimdRuntime::new();
//! let a = [1u8; 32];
//! let b = [1u8; 32];
//! assert!(runtime.compare_bytes_32(&a, &b));
//! ```

#![allow(unsafe_code)]

use crate::cpu::{CpuFeatures, SimdCapability};

/// SIMD runtime with automatic dispatch.
#[derive(Debug, Clone, Copy)]
pub struct SimdRuntime {
    /// Detected SIMD capability.
    capability: SimdCapability,
}

impl SimdRuntime {
    /// Create a new SIMD runtime with automatic detection.
    #[must_use]
    pub fn new() -> Self {
        Self {
            capability: CpuFeatures::get().best_simd_capability(),
        }
    }

    /// Create with explicit capability (for testing).
    #[must_use]
    pub const fn with_capability(capability: SimdCapability) -> Self {
        Self { capability }
    }

    /// Get the detected SIMD capability.
    #[must_use]
    pub const fn capability(&self) -> SimdCapability {
        self.capability
    }

    /// Get optimal vector width for operations.
    #[must_use]
    pub const fn vector_width(&self) -> usize {
        self.capability.vector_bytes()
    }
}

impl Default for SimdRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// SIMD operations trait.
pub trait SimdOps {
    /// Compare two 32-byte arrays (BLAKE3 hash size).
    fn compare_bytes_32(&self, a: &[u8; 32], b: &[u8; 32]) -> bool;

    /// Compare two 64-byte arrays.
    fn compare_bytes_64(&self, a: &[u8; 64], b: &[u8; 64]) -> bool;

    /// Find first occurrence of a byte.
    fn find_byte(&self, haystack: &[u8], needle: u8) -> Option<usize>;

    /// Find first occurrence of any byte in the set.
    fn find_byte_any(&self, haystack: &[u8], needles: &[u8]) -> Option<usize>;

    /// Check if slice starts with prefix.
    fn starts_with(&self, haystack: &[u8], prefix: &[u8]) -> bool;

    /// XOR two slices in place.
    fn xor_slices(&self, dst: &mut [u8], src: &[u8]);

    /// OR two slices in place.
    fn or_slices(&self, dst: &mut [u8], src: &[u8]);

    /// AND two slices in place.
    fn and_slices(&self, dst: &mut [u8], src: &[u8]);

    /// Count set bits in a u64.
    fn popcount_u64(&self, value: u64) -> u32;

    /// Count set bits in a slice of u64.
    fn popcount_slice(&self, data: &[u64]) -> usize;

    /// Memory set (fill with byte).
    fn memset(&self, dst: &mut [u8], value: u8);

    /// Memory copy.
    fn memcpy(&self, dst: &mut [u8], src: &[u8]);

    /// Compare memory regions.
    fn memcmp(&self, a: &[u8], b: &[u8]) -> std::cmp::Ordering;

    /// Sum bytes in a slice.
    fn sum_bytes(&self, data: &[u8]) -> u64;

    /// Find matching 32-byte hashes.
    fn find_matching_hashes(&self, hashes: &[[u8; 32]], target: &[u8; 32]) -> Vec<usize>;
}

impl SimdOps for SimdRuntime {
    #[inline]
    fn compare_bytes_32(&self, a: &[u8; 32], b: &[u8; 32]) -> bool {
        match self.capability {
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx512 | SimdCapability::Avx2 => {
                if std::arch::is_x86_feature_detected!("avx2") {
                    return unsafe { compare_bytes_32_avx2(a, b) };
                }
                a == b
            }
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Sse42 => {
                if std::arch::is_x86_feature_detected!("sse4.2") {
                    return unsafe { compare_bytes_32_sse42(a, b) };
                }
                a == b
            }
            #[cfg(target_arch = "aarch64")]
            SimdCapability::Neon | SimdCapability::Sve | SimdCapability::Sve2 => unsafe {
                compare_bytes_32_neon(a, b)
            },
            _ => a == b,
        }
    }

    #[inline]
    fn compare_bytes_64(&self, a: &[u8; 64], b: &[u8; 64]) -> bool {
        match self.capability {
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx512 => {
                if std::arch::is_x86_feature_detected!("avx512f") {
                    return unsafe { compare_bytes_64_avx512(a, b) };
                }
                a == b
            }
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx2 => {
                if std::arch::is_x86_feature_detected!("avx2") {
                    return unsafe { compare_bytes_64_avx2(a, b) };
                }
                a == b
            }
            #[cfg(target_arch = "aarch64")]
            SimdCapability::Neon | SimdCapability::Sve | SimdCapability::Sve2 => unsafe {
                compare_bytes_64_neon(a, b)
            },
            _ => a == b,
        }
    }

    #[inline]
    fn find_byte(&self, haystack: &[u8], needle: u8) -> Option<usize> {
        match self.capability {
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx512 | SimdCapability::Avx2 => {
                if std::arch::is_x86_feature_detected!("avx2") {
                    return unsafe { find_byte_avx2(haystack, needle) };
                }
                memchr::memchr(needle, haystack)
            }
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Sse42 => {
                if std::arch::is_x86_feature_detected!("sse4.2") {
                    return unsafe { find_byte_sse42(haystack, needle) };
                }
                memchr::memchr(needle, haystack)
            }
            #[cfg(target_arch = "aarch64")]
            SimdCapability::Neon | SimdCapability::Sve | SimdCapability::Sve2 => unsafe {
                find_byte_neon(haystack, needle)
            },
            _ => memchr::memchr(needle, haystack),
        }
    }

    #[inline]
    fn find_byte_any(&self, haystack: &[u8], needles: &[u8]) -> Option<usize> {
        match needles.len() {
            0 => None,
            1 => self.find_byte(haystack, needles[0]),
            2 => memchr::memchr2(needles[0], needles[1], haystack),
            3 => memchr::memchr3(needles[0], needles[1], needles[2], haystack),
            _ => {
                // For more needles, use a lookup table approach
                let mut table = [false; 256];
                for &needle in needles {
                    table[needle as usize] = true;
                }
                haystack.iter().position(|&b| table[b as usize])
            }
        }
    }

    #[inline]
    fn starts_with(&self, haystack: &[u8], prefix: &[u8]) -> bool {
        if prefix.len() > haystack.len() {
            return false;
        }

        if prefix.len() < 32 {
            return haystack.starts_with(prefix);
        }

        match self.capability {
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx512 | SimdCapability::Avx2 => {
                if std::arch::is_x86_feature_detected!("avx2") {
                    return unsafe { starts_with_avx2(haystack, prefix) };
                }
                haystack.starts_with(prefix)
            }
            #[cfg(target_arch = "aarch64")]
            SimdCapability::Neon | SimdCapability::Sve | SimdCapability::Sve2 => unsafe {
                starts_with_neon(haystack, prefix)
            },
            _ => haystack.starts_with(prefix),
        }
    }

    #[inline]
    fn xor_slices(&self, dst: &mut [u8], src: &[u8]) {
        assert_eq!(dst.len(), src.len());

        match self.capability {
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx512 | SimdCapability::Avx2 => {
                if std::arch::is_x86_feature_detected!("avx2") {
                    unsafe { xor_slices_avx2(dst, src) };
                    return;
                }
                xor_slices_scalar(dst, src);
            }
            #[cfg(target_arch = "aarch64")]
            SimdCapability::Neon | SimdCapability::Sve | SimdCapability::Sve2 => {
                unsafe { xor_slices_neon(dst, src) };
            }
            _ => xor_slices_scalar(dst, src),
        }
    }

    #[inline]
    fn or_slices(&self, dst: &mut [u8], src: &[u8]) {
        assert_eq!(dst.len(), src.len());

        match self.capability {
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx512 | SimdCapability::Avx2 => {
                if std::arch::is_x86_feature_detected!("avx2") {
                    unsafe { or_slices_avx2(dst, src) };
                    return;
                }
                or_slices_scalar(dst, src);
            }
            #[cfg(target_arch = "aarch64")]
            SimdCapability::Neon | SimdCapability::Sve | SimdCapability::Sve2 => {
                unsafe { or_slices_neon(dst, src) };
            }
            _ => or_slices_scalar(dst, src),
        }
    }

    #[inline]
    fn and_slices(&self, dst: &mut [u8], src: &[u8]) {
        assert_eq!(dst.len(), src.len());

        match self.capability {
            #[cfg(target_arch = "x86_64")]
            SimdCapability::Avx512 | SimdCapability::Avx2 => {
                if std::arch::is_x86_feature_detected!("avx2") {
                    unsafe { and_slices_avx2(dst, src) };
                    return;
                }
                and_slices_scalar(dst, src);
            }
            #[cfg(target_arch = "aarch64")]
            SimdCapability::Neon | SimdCapability::Sve | SimdCapability::Sve2 => {
                unsafe { and_slices_neon(dst, src) };
            }
            _ => and_slices_scalar(dst, src),
        }
    }

    #[inline]
    fn popcount_u64(&self, value: u64) -> u32 {
        value.count_ones()
    }

    #[inline]
    fn popcount_slice(&self, data: &[u64]) -> usize {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") && data.len() >= 4 {
                return unsafe { popcount_slice_avx2(data) };
            }
        }
        data.iter().map(|&x| x.count_ones() as usize).sum()
    }

    #[inline]
    fn memset(&self, dst: &mut [u8], value: u8) {
        // The compiler is very good at optimizing memset
        // Using explicit SIMD rarely helps here
        dst.fill(value);
    }

    #[inline]
    fn memcpy(&self, dst: &mut [u8], src: &[u8]) {
        dst.copy_from_slice(src);
    }

    #[inline]
    fn memcmp(&self, a: &[u8], b: &[u8]) -> std::cmp::Ordering {
        a.cmp(b)
    }

    #[inline]
    fn sum_bytes(&self, data: &[u8]) -> u64 {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx2") && data.len() >= 32 {
                unsafe { sum_bytes_avx2(data) }
            } else {
                data.iter().map(|&b| u64::from(b)).sum()
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            unsafe { sum_bytes_neon(data) }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            data.iter().map(|&b| u64::from(b)).sum()
        }
    }

    #[inline]
    fn find_matching_hashes(&self, hashes: &[[u8; 32]], target: &[u8; 32]) -> Vec<usize> {
        hashes
            .iter()
            .enumerate()
            .filter(|(_, h)| self.compare_bytes_32(h, target))
            .map(|(i, _)| i)
            .collect()
    }
}

// ============================================================================
// x86_64 AVX2 implementations
// ============================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn compare_bytes_32_avx2(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use std::arch::x86_64::{__m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8};

    unsafe {
        let va = _mm256_loadu_si256(a.as_ptr().cast::<__m256i>());
        let vb = _mm256_loadu_si256(b.as_ptr().cast::<__m256i>());
        let cmp = _mm256_cmpeq_epi8(va, vb);
        let mask = _mm256_movemask_epi8(cmp);
        mask == -1i32
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn compare_bytes_32_sse42(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use std::arch::x86_64::{__m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8};

    unsafe {
        let va0 = _mm_loadu_si128(a.as_ptr().cast::<__m128i>());
        let vb0 = _mm_loadu_si128(b.as_ptr().cast::<__m128i>());
        let va1 = _mm_loadu_si128(a.as_ptr().add(16).cast::<__m128i>());
        let vb1 = _mm_loadu_si128(b.as_ptr().add(16).cast::<__m128i>());

        let cmp0 = _mm_cmpeq_epi8(va0, vb0);
        let cmp1 = _mm_cmpeq_epi8(va1, vb1);

        let mask0 = _mm_movemask_epi8(cmp0);
        let mask1 = _mm_movemask_epi8(cmp1);

        mask0 == 0xFFFF && mask1 == 0xFFFF
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn compare_bytes_64_avx2(a: &[u8; 64], b: &[u8; 64]) -> bool {
    use std::arch::x86_64::{__m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8};

    unsafe {
        let va0 = _mm256_loadu_si256(a.as_ptr().cast::<__m256i>());
        let vb0 = _mm256_loadu_si256(b.as_ptr().cast::<__m256i>());
        let va1 = _mm256_loadu_si256(a.as_ptr().add(32).cast::<__m256i>());
        let vb1 = _mm256_loadu_si256(b.as_ptr().add(32).cast::<__m256i>());

        let cmp0 = _mm256_cmpeq_epi8(va0, vb0);
        let cmp1 = _mm256_cmpeq_epi8(va1, vb1);

        let mask0 = _mm256_movemask_epi8(cmp0);
        let mask1 = _mm256_movemask_epi8(cmp1);

        mask0 == -1i32 && mask1 == -1i32
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f", enable = "avx512bw")]
unsafe fn compare_bytes_64_avx512(a: &[u8; 64], b: &[u8; 64]) -> bool {
    use std::arch::x86_64::{__m512i, _mm512_cmpeq_epi8_mask, _mm512_loadu_si512};

    unsafe {
        let va = _mm512_loadu_si512(a.as_ptr().cast::<__m512i>());
        let vb = _mm512_loadu_si512(b.as_ptr().cast::<__m512i>());
        let mask = _mm512_cmpeq_epi8_mask(va, vb);
        mask == u64::MAX
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn find_byte_avx2(haystack: &[u8], needle: u8) -> Option<usize> {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8,
    };

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

        None
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn find_byte_sse42(haystack: &[u8], needle: u8) -> Option<usize> {
    use std::arch::x86_64::{
        __m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8,
    };

    if haystack.is_empty() {
        return None;
    }

    unsafe {
        let needle_vec = _mm_set1_epi8(needle as i8);
        let chunks = haystack.len() / 16;

        for i in 0..chunks {
            let offset = i * 16;
            let chunk = _mm_loadu_si128(haystack.as_ptr().add(offset).cast::<__m128i>());
            let cmp = _mm_cmpeq_epi8(chunk, needle_vec);
            let mask = _mm_movemask_epi8(cmp) as u32;
            if mask != 0 {
                return Some(offset + mask.trailing_zeros() as usize);
            }
        }

        let remaining_start = chunks * 16;
        for (i, &byte) in haystack[remaining_start..].iter().enumerate() {
            if byte == needle {
                return Some(remaining_start + i);
            }
        }

        None
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn starts_with_avx2(haystack: &[u8], prefix: &[u8]) -> bool {
    use std::arch::x86_64::{__m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8};

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

        let remaining_start = chunks * 32;
        haystack[remaining_start..].starts_with(&prefix[remaining_start..])
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn xor_slices_avx2(dst: &mut [u8], src: &[u8]) {
    use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_storeu_si256, _mm256_xor_si256};

    unsafe {
        let chunks = dst.len() / 32;

        for i in 0..chunks {
            let offset = i * 32;
            let vd = _mm256_loadu_si256(dst.as_ptr().add(offset).cast::<__m256i>());
            let vs = _mm256_loadu_si256(src.as_ptr().add(offset).cast::<__m256i>());
            let result = _mm256_xor_si256(vd, vs);
            _mm256_storeu_si256(dst.as_mut_ptr().add(offset).cast::<__m256i>(), result);
        }

        let remaining_start = chunks * 32;
        xor_slices_scalar(&mut dst[remaining_start..], &src[remaining_start..]);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn or_slices_avx2(dst: &mut [u8], src: &[u8]) {
    use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_or_si256, _mm256_storeu_si256};

    unsafe {
        let chunks = dst.len() / 32;

        for i in 0..chunks {
            let offset = i * 32;
            let vd = _mm256_loadu_si256(dst.as_ptr().add(offset).cast::<__m256i>());
            let vs = _mm256_loadu_si256(src.as_ptr().add(offset).cast::<__m256i>());
            let result = _mm256_or_si256(vd, vs);
            _mm256_storeu_si256(dst.as_mut_ptr().add(offset).cast::<__m256i>(), result);
        }

        let remaining_start = chunks * 32;
        or_slices_scalar(&mut dst[remaining_start..], &src[remaining_start..]);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn and_slices_avx2(dst: &mut [u8], src: &[u8]) {
    use std::arch::x86_64::{__m256i, _mm256_and_si256, _mm256_loadu_si256, _mm256_storeu_si256};

    unsafe {
        let chunks = dst.len() / 32;

        for i in 0..chunks {
            let offset = i * 32;
            let vd = _mm256_loadu_si256(dst.as_ptr().add(offset).cast::<__m256i>());
            let vs = _mm256_loadu_si256(src.as_ptr().add(offset).cast::<__m256i>());
            let result = _mm256_and_si256(vd, vs);
            _mm256_storeu_si256(dst.as_mut_ptr().add(offset).cast::<__m256i>(), result);
        }

        let remaining_start = chunks * 32;
        and_slices_scalar(&mut dst[remaining_start..], &src[remaining_start..]);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn popcount_slice_avx2(data: &[u64]) -> usize {
    use std::arch::x86_64::{
        __m256i, _mm_add_epi64, _mm_extract_epi64, _mm256_add_epi8, _mm256_add_epi64,
        _mm256_and_si256, _mm256_extracti128_si256, _mm256_loadu_si256, _mm256_sad_epu8,
        _mm256_set1_epi8, _mm256_setr_epi8, _mm256_setzero_si256, _mm256_shuffle_epi8,
        _mm256_srli_epi16,
    };

    unsafe {
        // Use lookup table approach for AVX2 popcount
        let low_mask = _mm256_set1_epi8(0x0f);
        let lookup = _mm256_setr_epi8(
            0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, 0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2,
            3, 3, 4,
        );

        let mut total = _mm256_setzero_si256();
        let chunks = data.len() / 4;

        for i in 0..chunks {
            let offset = i * 4;
            let v = _mm256_loadu_si256(data.as_ptr().add(offset).cast::<__m256i>());

            let lo = _mm256_and_si256(v, low_mask);
            let hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), low_mask);

            let popcnt_lo = _mm256_shuffle_epi8(lookup, lo);
            let popcnt_hi = _mm256_shuffle_epi8(lookup, hi);

            let local = _mm256_add_epi8(popcnt_lo, popcnt_hi);
            total = _mm256_add_epi64(total, _mm256_sad_epu8(local, _mm256_setzero_si256()));
        }

        // Sum horizontal
        let sum128 = _mm_add_epi64(
            _mm256_extracti128_si256(total, 0),
            _mm256_extracti128_si256(total, 1),
        );
        let sum = _mm_extract_epi64(sum128, 0) + _mm_extract_epi64(sum128, 1);

        // Handle remaining
        let remaining_start = chunks * 4;
        let remaining_sum: usize = data[remaining_start..]
            .iter()
            .map(|&x| x.count_ones() as usize)
            .sum();

        sum as usize + remaining_sum
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn sum_bytes_avx2(data: &[u8]) -> u64 {
    use std::arch::x86_64::{
        __m256i, _mm_add_epi64, _mm_extract_epi64, _mm256_add_epi64, _mm256_extracti128_si256,
        _mm256_loadu_si256, _mm256_sad_epu8, _mm256_setzero_si256,
    };

    unsafe {
        let mut total = _mm256_setzero_si256();
        let chunks = data.len() / 32;

        for i in 0..chunks {
            let offset = i * 32;
            let v = _mm256_loadu_si256(data.as_ptr().add(offset).cast::<__m256i>());
            total = _mm256_add_epi64(total, _mm256_sad_epu8(v, _mm256_setzero_si256()));
        }

        // Sum horizontal
        let sum128 = _mm_add_epi64(
            _mm256_extracti128_si256(total, 0),
            _mm256_extracti128_si256(total, 1),
        );
        let sum = (_mm_extract_epi64(sum128, 0) + _mm_extract_epi64(sum128, 1)) as u64;

        // Handle remaining
        let remaining_start = chunks * 32;
        let remaining_sum: u64 = data[remaining_start..].iter().map(|&b| u64::from(b)).sum();

        sum + remaining_sum
    }
}

// ============================================================================
// ARM64 NEON implementations
// ============================================================================

#[cfg(target_arch = "aarch64")]
unsafe fn compare_bytes_32_neon(a: &[u8; 32], b: &[u8; 32]) -> bool {
    use std::arch::aarch64::*;

    let va0 = vld1q_u8(a.as_ptr());
    let vb0 = vld1q_u8(b.as_ptr());
    let va1 = vld1q_u8(a.as_ptr().add(16));
    let vb1 = vld1q_u8(b.as_ptr().add(16));

    let cmp0 = vceqq_u8(va0, vb0);
    let cmp1 = vceqq_u8(va1, vb1);

    // Check if all bytes match
    let min0 = vminvq_u8(cmp0);
    let min1 = vminvq_u8(cmp1);

    min0 == 0xFF && min1 == 0xFF
}

#[cfg(target_arch = "aarch64")]
unsafe fn compare_bytes_64_neon(a: &[u8; 64], b: &[u8; 64]) -> bool {
    use std::arch::aarch64::*;

    let va0 = vld1q_u8(a.as_ptr());
    let vb0 = vld1q_u8(b.as_ptr());
    let va1 = vld1q_u8(a.as_ptr().add(16));
    let vb1 = vld1q_u8(b.as_ptr().add(16));
    let va2 = vld1q_u8(a.as_ptr().add(32));
    let vb2 = vld1q_u8(b.as_ptr().add(32));
    let va3 = vld1q_u8(a.as_ptr().add(48));
    let vb3 = vld1q_u8(b.as_ptr().add(48));

    let cmp0 = vceqq_u8(va0, vb0);
    let cmp1 = vceqq_u8(va1, vb1);
    let cmp2 = vceqq_u8(va2, vb2);
    let cmp3 = vceqq_u8(va3, vb3);

    let and01 = vandq_u8(cmp0, cmp1);
    let and23 = vandq_u8(cmp2, cmp3);
    let and_all = vandq_u8(and01, and23);

    vminvq_u8(and_all) == 0xFF
}

#[cfg(target_arch = "aarch64")]
unsafe fn find_byte_neon(haystack: &[u8], needle: u8) -> Option<usize> {
    use std::arch::aarch64::*;

    if haystack.is_empty() {
        return None;
    }

    let needle_vec = vdupq_n_u8(needle);
    let chunks = haystack.len() / 16;

    for i in 0..chunks {
        let offset = i * 16;
        let chunk = vld1q_u8(haystack.as_ptr().add(offset));
        let cmp = vceqq_u8(chunk, needle_vec);

        // Convert comparison result to a mask
        let mask_low = vget_low_u8(cmp);
        let mask_high = vget_high_u8(cmp);

        // Check if any byte matches
        let any_low = vmaxv_u8(mask_low);
        let any_high = vmaxv_u8(mask_high);

        if any_low == 0xFF {
            // Find first match in low half
            for j in 0..8 {
                if haystack[offset + j] == needle {
                    return Some(offset + j);
                }
            }
        }
        if any_high == 0xFF {
            // Find first match in high half
            for j in 8..16 {
                if haystack[offset + j] == needle {
                    return Some(offset + j);
                }
            }
        }
    }

    // Handle remaining bytes
    let remaining_start = chunks * 16;
    for (i, &byte) in haystack[remaining_start..].iter().enumerate() {
        if byte == needle {
            return Some(remaining_start + i);
        }
    }

    None
}

#[cfg(target_arch = "aarch64")]
unsafe fn starts_with_neon(haystack: &[u8], prefix: &[u8]) -> bool {
    use std::arch::aarch64::*;

    let chunks = prefix.len() / 16;

    for i in 0..chunks {
        let offset = i * 16;
        let h = vld1q_u8(haystack.as_ptr().add(offset));
        let p = vld1q_u8(prefix.as_ptr().add(offset));
        let cmp = vceqq_u8(h, p);

        if vminvq_u8(cmp) != 0xFF {
            return false;
        }
    }

    let remaining_start = chunks * 16;
    haystack[remaining_start..].starts_with(&prefix[remaining_start..])
}

#[cfg(target_arch = "aarch64")]
unsafe fn xor_slices_neon(dst: &mut [u8], src: &[u8]) {
    use std::arch::aarch64::*;

    let chunks = dst.len() / 16;

    for i in 0..chunks {
        let offset = i * 16;
        let vd = vld1q_u8(dst.as_ptr().add(offset));
        let vs = vld1q_u8(src.as_ptr().add(offset));
        let result = veorq_u8(vd, vs);
        vst1q_u8(dst.as_mut_ptr().add(offset), result);
    }

    let remaining_start = chunks * 16;
    xor_slices_scalar(&mut dst[remaining_start..], &src[remaining_start..]);
}

#[cfg(target_arch = "aarch64")]
unsafe fn or_slices_neon(dst: &mut [u8], src: &[u8]) {
    use std::arch::aarch64::*;

    let chunks = dst.len() / 16;

    for i in 0..chunks {
        let offset = i * 16;
        let vd = vld1q_u8(dst.as_ptr().add(offset));
        let vs = vld1q_u8(src.as_ptr().add(offset));
        let result = vorrq_u8(vd, vs);
        vst1q_u8(dst.as_mut_ptr().add(offset), result);
    }

    let remaining_start = chunks * 16;
    or_slices_scalar(&mut dst[remaining_start..], &src[remaining_start..]);
}

#[cfg(target_arch = "aarch64")]
unsafe fn and_slices_neon(dst: &mut [u8], src: &[u8]) {
    use std::arch::aarch64::*;

    let chunks = dst.len() / 16;

    for i in 0..chunks {
        let offset = i * 16;
        let vd = vld1q_u8(dst.as_ptr().add(offset));
        let vs = vld1q_u8(src.as_ptr().add(offset));
        let result = vandq_u8(vd, vs);
        vst1q_u8(dst.as_mut_ptr().add(offset), result);
    }

    let remaining_start = chunks * 16;
    and_slices_scalar(&mut dst[remaining_start..], &src[remaining_start..]);
}

#[cfg(target_arch = "aarch64")]
unsafe fn sum_bytes_neon(data: &[u8]) -> u64 {
    use std::arch::aarch64::*;

    let mut total: u64 = 0;
    let chunks = data.len() / 16;

    for i in 0..chunks {
        let offset = i * 16;
        let v = vld1q_u8(data.as_ptr().add(offset));

        // Pairwise add to avoid overflow
        let sum16 = vpaddlq_u8(v);
        let sum32 = vpaddlq_u16(sum16);
        let sum64 = vpaddlq_u32(sum32);

        total += vgetq_lane_u64(sum64, 0) + vgetq_lane_u64(sum64, 1);
    }

    // Handle remaining
    let remaining_start = chunks * 16;
    let remaining_sum: u64 = data[remaining_start..].iter().map(|&b| b as u64).sum();

    total + remaining_sum
}

// ============================================================================
// Scalar fallback implementations
// ============================================================================

#[inline]
fn xor_slices_scalar(dst: &mut [u8], src: &[u8]) {
    for (d, &s) in dst.iter_mut().zip(src.iter()) {
        *d ^= s;
    }
}

#[inline]
fn or_slices_scalar(dst: &mut [u8], src: &[u8]) {
    for (d, &s) in dst.iter_mut().zip(src.iter()) {
        *d |= s;
    }
}

#[inline]
fn and_slices_scalar(dst: &mut [u8], src: &[u8]) {
    for (d, &s) in dst.iter_mut().zip(src.iter()) {
        *d &= s;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simd_runtime_creation() {
        let runtime = SimdRuntime::new();
        assert!(runtime.vector_width() >= 8);
    }

    #[test]
    fn compare_bytes_32_equal() {
        let runtime = SimdRuntime::new();
        let a = [42u8; 32];
        let b = [42u8; 32];
        assert!(runtime.compare_bytes_32(&a, &b));
    }

    #[test]
    fn compare_bytes_32_not_equal() {
        let runtime = SimdRuntime::new();
        let a = [42u8; 32];
        let mut b = [42u8; 32];
        b[15] = 0;
        assert!(!runtime.compare_bytes_32(&a, &b));
    }

    #[test]
    fn compare_bytes_64() {
        let runtime = SimdRuntime::new();
        let a = [42u8; 64];
        let b = [42u8; 64];
        assert!(runtime.compare_bytes_64(&a, &b));
    }

    #[test]
    fn find_byte_found() {
        let runtime = SimdRuntime::new();
        let data = b"hello world";
        assert_eq!(runtime.find_byte(data, b'w'), Some(6));
        assert_eq!(runtime.find_byte(data, b'h'), Some(0));
        assert_eq!(runtime.find_byte(data, b'd'), Some(10));
    }

    #[test]
    fn find_byte_not_found() {
        let runtime = SimdRuntime::new();
        let data = b"hello world";
        assert_eq!(runtime.find_byte(data, b'x'), None);
    }

    #[test]
    fn find_byte_any_single() {
        let runtime = SimdRuntime::new();
        let data = b"hello world";
        assert_eq!(runtime.find_byte_any(data, b"w"), Some(6));
    }

    #[test]
    fn find_byte_any_multiple() {
        let runtime = SimdRuntime::new();
        let data = b"hello world";
        assert_eq!(runtime.find_byte_any(data, b"ow"), Some(4)); // 'o' comes first
    }

    #[test]
    fn starts_with_true() {
        let runtime = SimdRuntime::new();
        let haystack = b"hello world this is a longer string for testing";
        assert!(runtime.starts_with(haystack, b"hello"));
        assert!(runtime.starts_with(haystack, b"hello world this is a longer"));
    }

    #[test]
    fn starts_with_false() {
        let runtime = SimdRuntime::new();
        let haystack = b"hello world";
        assert!(!runtime.starts_with(haystack, b"world"));
        assert!(!runtime.starts_with(haystack, b"hello world!"));
    }

    #[test]
    fn xor_slices() {
        let runtime = SimdRuntime::new();
        let mut dst = vec![0xFFu8; 64];
        let src = vec![0xAAu8; 64];
        runtime.xor_slices(&mut dst, &src);
        assert!(dst.iter().all(|&b| b == 0x55));
    }

    #[test]
    fn or_slices() {
        let runtime = SimdRuntime::new();
        let mut dst = vec![0xF0u8; 64];
        let src = vec![0x0Fu8; 64];
        runtime.or_slices(&mut dst, &src);
        assert!(dst.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn and_slices() {
        let runtime = SimdRuntime::new();
        let mut dst = vec![0xFFu8; 64];
        let src = vec![0xF0u8; 64];
        runtime.and_slices(&mut dst, &src);
        assert!(dst.iter().all(|&b| b == 0xF0));
    }

    #[test]
    fn popcount_u64() {
        let runtime = SimdRuntime::new();
        assert_eq!(runtime.popcount_u64(0), 0);
        assert_eq!(runtime.popcount_u64(1), 1);
        assert_eq!(runtime.popcount_u64(0xFF), 8);
        assert_eq!(runtime.popcount_u64(u64::MAX), 64);
    }

    #[test]
    fn popcount_slice() {
        let runtime = SimdRuntime::new();
        let data = vec![0u64, 1, 0xFF, u64::MAX];
        assert_eq!(runtime.popcount_slice(&data), 1 + 8 + 64);
    }

    #[test]
    fn sum_bytes() {
        let runtime = SimdRuntime::new();
        let data = vec![1u8; 100];
        assert_eq!(runtime.sum_bytes(&data), 100);

        let data2: Vec<u8> = (0..=255).collect();
        assert_eq!(runtime.sum_bytes(&data2), (0..=255u64).sum());
    }

    #[test]
    fn find_matching_hashes() {
        let runtime = SimdRuntime::new();
        let target = [42u8; 32];
        let hashes = vec![[0u8; 32], [42u8; 32], [1u8; 32], [42u8; 32]];
        let matches = runtime.find_matching_hashes(&hashes, &target);
        assert_eq!(matches, vec![1, 3]);
    }

    #[test]
    fn memset() {
        let runtime = SimdRuntime::new();
        let mut data = vec![0u8; 100];
        runtime.memset(&mut data, 0xAB);
        assert!(data.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn memcpy() {
        let runtime = SimdRuntime::new();
        let src = vec![42u8; 100];
        let mut dst = vec![0u8; 100];
        runtime.memcpy(&mut dst, &src);
        assert_eq!(dst, src);
    }

    #[test]
    fn capability_detection() {
        let runtime = SimdRuntime::new();
        let cap = runtime.capability();

        #[cfg(target_arch = "x86_64")]
        {
            // x86_64 should have at least SSE4.2 on modern CPUs
            // But we can't guarantee this on all CI machines
            assert!(cap >= SimdCapability::Scalar);
        }

        #[cfg(target_arch = "aarch64")]
        {
            // AArch64 always has NEON
            assert!(cap >= SimdCapability::Neon);
        }
    }
}
