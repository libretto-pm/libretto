//! Ultra-high-performance Bloom Filter implementation
//!
//! Features:
//! - MurmurHash64A with double hashing for k hash functions
//! - Scalable Bloom filters with sub-filter stacking
//! - Cache-friendly 64-bit aligned bit array
//! - O(k) add/check operations
//!
//! Based on Redis Bloom filter design for full compatibility.

use parking_lot::RwLock;
use std::f64::consts::LN_2;
use std::hash::Hash;

// ============================================================================
// MurmurHash64A Implementation
// ============================================================================

/// MurmurHash64A - fast, high-quality 64-bit hash
/// Used by Redis for Bloom filters and hash tables
#[inline]
fn murmurhash64a(data: &[u8], seed: u64) -> u64 {
    const M: u64 = 0xc6a4a7935bd1e995;
    const R: i32 = 47;

    let len = data.len();
    let mut h: u64 = seed ^ ((len as u64).wrapping_mul(M));

    // Process 8-byte chunks
    let chunks = len / 8;
    for i in 0..chunks {
        let offset = i * 8;
        let mut k = u64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);

        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);

        h ^= k;
        h = h.wrapping_mul(M);
    }

    // Handle remaining bytes
    let remaining = &data[chunks * 8..];
    match remaining.len() {
        7 => {
            h ^= (remaining[6] as u64) << 48;
            h ^= (remaining[5] as u64) << 40;
            h ^= (remaining[4] as u64) << 32;
            h ^= (remaining[3] as u64) << 24;
            h ^= (remaining[2] as u64) << 16;
            h ^= (remaining[1] as u64) << 8;
            h ^= remaining[0] as u64;
            h = h.wrapping_mul(M);
        }
        6 => {
            h ^= (remaining[5] as u64) << 40;
            h ^= (remaining[4] as u64) << 32;
            h ^= (remaining[3] as u64) << 24;
            h ^= (remaining[2] as u64) << 16;
            h ^= (remaining[1] as u64) << 8;
            h ^= remaining[0] as u64;
            h = h.wrapping_mul(M);
        }
        5 => {
            h ^= (remaining[4] as u64) << 32;
            h ^= (remaining[3] as u64) << 24;
            h ^= (remaining[2] as u64) << 16;
            h ^= (remaining[1] as u64) << 8;
            h ^= remaining[0] as u64;
            h = h.wrapping_mul(M);
        }
        4 => {
            h ^= (remaining[3] as u64) << 24;
            h ^= (remaining[2] as u64) << 16;
            h ^= (remaining[1] as u64) << 8;
            h ^= remaining[0] as u64;
            h = h.wrapping_mul(M);
        }
        3 => {
            h ^= (remaining[2] as u64) << 16;
            h ^= (remaining[1] as u64) << 8;
            h ^= remaining[0] as u64;
            h = h.wrapping_mul(M);
        }
        2 => {
            h ^= (remaining[1] as u64) << 8;
            h ^= remaining[0] as u64;
            h = h.wrapping_mul(M);
        }
        1 => {
            h ^= remaining[0] as u64;
            h = h.wrapping_mul(M);
        }
        _ => {}
    }

    h ^= h >> R;
    h = h.wrapping_mul(M);
    h ^= h >> R;

    h
}

/// Hash any hashable type to bytes for the bloom filter
#[inline]
fn hash_to_bytes<T: Hash>(item: &T) -> Vec<u8> {
    use std::hash::Hasher;
    let mut hasher = ahash::AHasher::default();
    item.hash(&mut hasher);
    hasher.finish().to_le_bytes().to_vec()
}

// ============================================================================
// Bloom Filter Configuration
// ============================================================================

/// Configuration for a Bloom filter
#[derive(Debug, Clone)]
pub struct BloomFilterConfig {
    /// Target false positive rate (e.g., 0.01 = 1%)
    pub error_rate: f64,
    /// Expected number of items
    pub capacity: usize,
    /// Expansion factor for scaling (default: 2)
    pub expansion: u32,
    /// If true, don't create new sub-filters when full
    pub nonscaling: bool,
}

impl Default for BloomFilterConfig {
    fn default() -> Self {
        Self {
            error_rate: 0.01,
            capacity: 100_000,
            expansion: 2,
            nonscaling: false,
        }
    }
}

impl BloomFilterConfig {
    /// Calculate optimal number of hash functions
    #[inline]
    pub fn optimal_hash_count(error_rate: f64) -> u32 {
        ((-error_rate.ln() / LN_2).ceil() as u32).max(1)
    }

    /// Calculate optimal number of bits per item
    #[inline]
    pub fn bits_per_item(error_rate: f64) -> f64 {
        -error_rate.ln() / (LN_2 * LN_2)
    }

    /// Calculate required bit array size
    #[inline]
    pub fn required_bits(capacity: usize, error_rate: f64) -> usize {
        let bits_per_item = Self::bits_per_item(error_rate);
        ((capacity as f64 * bits_per_item).ceil() as usize).max(64)
    }
}

// ============================================================================
// Single Bloom Filter (Non-scaling)
// ============================================================================

/// A single Bloom filter with fixed capacity
#[derive(Debug, Clone)]
pub struct BloomFilter {
    /// Bit array stored as u64s for cache efficiency
    bits: Vec<u64>,
    /// Total number of bits
    num_bits: usize,
    /// Number of hash functions
    num_hashes: u32,
    /// Number of items added
    items_added: usize,
    /// Configured capacity
    capacity: usize,
    /// Error rate for this filter
    error_rate: f64,
}

impl BloomFilter {
    /// Create a new Bloom filter with given capacity and error rate
    #[must_use]
    pub fn new(capacity: usize, error_rate: f64) -> Self {
        let num_bits = BloomFilterConfig::required_bits(capacity, error_rate);
        let num_hashes = BloomFilterConfig::optimal_hash_count(error_rate);

        // Round up to next u64 boundary
        let num_u64s = (num_bits + 63) / 64;
        let actual_bits = num_u64s * 64;

        Self {
            bits: vec![0u64; num_u64s],
            num_bits: actual_bits,
            num_hashes,
            items_added: 0,
            capacity,
            error_rate,
        }
    }

    /// Add an item to the filter using bytes. Returns true if the item might be new.
    #[inline]
    pub fn add_bytes(&mut self, item: &[u8]) -> bool {
        let hash = murmurhash64a(item, 0);
        let h1 = (hash >> 32) as u32;
        let h2 = hash as u32;

        let mut possibly_new = false;

        for i in 0..self.num_hashes {
            // Double hashing: h(i) = h1 + i * h2
            let combined = (h1 as u64).wrapping_add((i as u64).wrapping_mul(h2 as u64));
            let bit_index = (combined % self.num_bits as u64) as usize;

            let word_index = bit_index / 64;
            let bit_offset = bit_index % 64;
            let mask = 1u64 << bit_offset;

            if self.bits[word_index] & mask == 0 {
                possibly_new = true;
            }
            self.bits[word_index] |= mask;
        }

        if possibly_new {
            self.items_added += 1;
        }

        possibly_new
    }

    /// Add any hashable item to the filter
    #[inline]
    pub fn insert<T: Hash>(&mut self, item: &T) -> bool {
        let bytes = hash_to_bytes(item);
        self.add_bytes(&bytes)
    }

    /// Check if bytes might exist in the filter
    #[inline]
    pub fn exists_bytes(&self, item: &[u8]) -> bool {
        let hash = murmurhash64a(item, 0);
        let h1 = (hash >> 32) as u32;
        let h2 = hash as u32;

        for i in 0..self.num_hashes {
            let combined = (h1 as u64).wrapping_add((i as u64).wrapping_mul(h2 as u64));
            let bit_index = (combined % self.num_bits as u64) as usize;

            let word_index = bit_index / 64;
            let bit_offset = bit_index % 64;
            let mask = 1u64 << bit_offset;

            if self.bits[word_index] & mask == 0 {
                return false;
            }
        }

        true
    }

    /// Check if any hashable item might exist in the filter
    #[inline]
    pub fn may_contain<T: Hash>(&self, item: &T) -> bool {
        let bytes = hash_to_bytes(item);
        self.exists_bytes(&bytes)
    }

    /// Check if the filter should be scaled (capacity exceeded)
    #[inline]
    pub fn should_scale(&self) -> bool {
        self.items_added >= self.capacity
    }

    /// Get the number of items added
    #[inline]
    #[must_use]
    pub fn count(&self) -> u64 {
        self.items_added as u64
    }

    /// Get the number of items added
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.items_added
    }

    /// Check if filter is empty
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items_added == 0
    }

    /// Get capacity
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the size in bytes
    #[inline]
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.bits.len() * 8
    }

    /// Get number of bits
    #[inline]
    #[must_use]
    pub fn num_bits(&self) -> usize {
        self.num_bits
    }

    /// Get number of hash functions
    #[inline]
    #[must_use]
    pub fn num_hashes(&self) -> usize {
        self.num_hashes as usize
    }

    /// Get error rate
    #[inline]
    #[must_use]
    pub fn error_rate(&self) -> f64 {
        self.error_rate
    }

    /// Clear the bloom filter
    pub fn clear(&self) {
        // Note: This is a no-op for non-mutable clear
        // For actual clearing, use clear_mut
    }

    /// Clear the bloom filter (mutable)
    pub fn clear_mut(&mut self) {
        for word in &mut self.bits {
            *word = 0;
        }
        self.items_added = 0;
    }

    /// Get the estimated false positive rate based on current fill
    #[must_use]
    pub fn estimated_fp_rate(&self) -> f64 {
        let bits_set = self.count_bits_set();
        let fill_ratio = bits_set as f64 / self.num_bits as f64;
        fill_ratio.powi(self.num_hashes as i32)
    }

    fn count_bits_set(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Serialize the filter for persistence
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(32 + self.bits.len() * 8);

        // Header: num_bits (8) + num_hashes (4) + items_added (8) + capacity (8) + error_rate (8)
        result.extend_from_slice(&(self.num_bits as u64).to_le_bytes());
        result.extend_from_slice(&self.num_hashes.to_le_bytes());
        result.extend_from_slice(&(self.items_added as u64).to_le_bytes());
        result.extend_from_slice(&(self.capacity as u64).to_le_bytes());
        result.extend_from_slice(&self.error_rate.to_le_bytes());

        // Bit array
        for word in &self.bits {
            result.extend_from_slice(&word.to_le_bytes());
        }

        result
    }

    /// Deserialize the filter
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 36 {
            return None;
        }

        let num_bits = u64::from_le_bytes(data[0..8].try_into().ok()?) as usize;
        let num_hashes = u32::from_le_bytes(data[8..12].try_into().ok()?);
        let items_added = u64::from_le_bytes(data[12..20].try_into().ok()?) as usize;
        let capacity = u64::from_le_bytes(data[20..28].try_into().ok()?) as usize;
        let error_rate = f64::from_le_bytes(data[28..36].try_into().ok()?);

        let num_u64s = (num_bits + 63) / 64;
        let expected_len = 36 + num_u64s * 8;
        if data.len() < expected_len {
            return None;
        }

        let mut bits = Vec::with_capacity(num_u64s);
        for i in 0..num_u64s {
            let offset = 36 + i * 8;
            let word = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            bits.push(word);
        }

        Some(Self {
            bits,
            num_bits,
            num_hashes,
            items_added,
            capacity,
            error_rate,
        })
    }
}

// ============================================================================
// Scalable Bloom Filter
// ============================================================================

/// A scalable Bloom filter that grows by adding sub-filters
#[derive(Debug, Clone)]
pub struct ScalableBloomFilter {
    /// Stack of filters (newest last)
    filters: Vec<BloomFilter>,
    /// Configuration
    pub config: BloomFilterConfig,
    /// Total items added across all filters
    total_items: usize,
}

impl ScalableBloomFilter {
    /// Create a new scalable Bloom filter
    #[must_use]
    pub fn new(config: BloomFilterConfig) -> Self {
        let initial_filter = BloomFilter::new(config.capacity, config.error_rate);
        Self {
            filters: vec![initial_filter],
            config,
            total_items: 0,
        }
    }

    /// Create with specific error rate and capacity
    #[must_use]
    pub fn with_capacity(capacity: usize, error_rate: f64) -> Self {
        Self::new(BloomFilterConfig {
            capacity,
            error_rate,
            ..Default::default()
        })
    }

    /// Add bytes. Returns true if the item might be new.
    pub fn add_bytes(&mut self, item: &[u8]) -> bool {
        // First check if item already exists
        if self.exists_bytes(item) {
            return false;
        }

        // Get the current (last) filter
        let current = self.filters.last_mut().unwrap();

        // Check if we need to scale
        if current.should_scale() && !self.config.nonscaling {
            // Create a new filter with expanded capacity
            let new_capacity = current.capacity() * self.config.expansion as usize;
            // Use tighter error rate for new filters to maintain overall error rate
            let new_error_rate = current.error_rate() * 0.5;
            let new_filter = BloomFilter::new(new_capacity, new_error_rate);
            self.filters.push(new_filter);
        }

        // Add to the current (potentially new) filter
        let current = self.filters.last_mut().unwrap();
        let added = current.add_bytes(item);
        if added {
            self.total_items += 1;
        }
        added
    }

    /// Add any hashable item
    pub fn insert<T: Hash>(&mut self, item: &T) -> bool {
        let bytes = hash_to_bytes(item);
        self.add_bytes(&bytes)
    }

    /// Check if bytes might exist
    #[must_use]
    pub fn exists_bytes(&self, item: &[u8]) -> bool {
        // Check all filters from newest to oldest
        for filter in self.filters.iter().rev() {
            if filter.exists_bytes(item) {
                return true;
            }
        }
        false
    }

    /// Check if any hashable item might exist
    #[must_use]
    pub fn may_contain<T: Hash>(&self, item: &T) -> bool {
        let bytes = hash_to_bytes(item);
        self.exists_bytes(&bytes)
    }

    /// Get total items added
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.total_items
    }

    /// Check if empty
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total_items == 0
    }

    /// Get configured capacity
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.config.capacity
    }

    /// Get number of sub-filters
    #[inline]
    #[must_use]
    pub fn num_filters(&self) -> usize {
        self.filters.len()
    }

    /// Get total size in bytes
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.filters.iter().map(BloomFilter::memory_usage).sum()
    }

    /// Get error rate
    #[inline]
    #[must_use]
    pub fn error_rate(&self) -> f64 {
        self.config.error_rate
    }

    /// Clear all filters
    pub fn clear(&mut self) {
        self.filters.clear();
        self.filters.push(BloomFilter::new(
            self.config.capacity,
            self.config.error_rate,
        ));
        self.total_items = 0;
    }

    /// Serialize for persistence
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::new();

        // Header: version (1) + num_filters (4) + config
        result.push(1); // version
        result.extend_from_slice(&(self.filters.len() as u32).to_le_bytes());
        result.extend_from_slice(&self.config.error_rate.to_le_bytes());
        result.extend_from_slice(&(self.config.capacity as u64).to_le_bytes());
        result.extend_from_slice(&self.config.expansion.to_le_bytes());
        result.push(u8::from(self.config.nonscaling));
        result.extend_from_slice(&(self.total_items as u64).to_le_bytes());

        // Each filter with length prefix
        for filter in &self.filters {
            let filter_bytes = filter.to_bytes();
            result.extend_from_slice(&(filter_bytes.len() as u32).to_le_bytes());
            result.extend_from_slice(&filter_bytes);
        }

        result
    }

    /// Deserialize from bytes
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 30 {
            return None;
        }

        let version = data[0];
        if version != 1 {
            return None;
        }

        let num_filters = u32::from_le_bytes(data[1..5].try_into().ok()?) as usize;
        let error_rate = f64::from_le_bytes(data[5..13].try_into().ok()?);
        let capacity = u64::from_le_bytes(data[13..21].try_into().ok()?) as usize;
        let expansion = u32::from_le_bytes(data[21..25].try_into().ok()?);
        let nonscaling = data[25] != 0;
        let total_items = u64::from_le_bytes(data[26..34].try_into().ok()?) as usize;

        let config = BloomFilterConfig {
            error_rate,
            capacity,
            expansion,
            nonscaling,
        };

        let mut filters = Vec::with_capacity(num_filters);
        let mut offset = 34;

        for _ in 0..num_filters {
            if offset + 4 > data.len() {
                return None;
            }
            let filter_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
            offset += 4;
            if offset + filter_len > data.len() {
                return None;
            }
            let filter = BloomFilter::from_bytes(&data[offset..offset + filter_len])?;
            filters.push(filter);
            offset += filter_len;
        }

        Some(Self {
            filters,
            config,
            total_items,
        })
    }
}

// ============================================================================
// Thread-safe Concurrent Bloom Filter
// ============================================================================

/// Magic bytes for bloom filter persistence.
const BLOOM_MAGIC: &[u8; 4] = b"BLMF";

/// Thread-safe bloom filter with rebuild capability and persistence
#[derive(Debug)]
pub struct ConcurrentBloomFilter {
    filter: RwLock<ScalableBloomFilter>,
    expected_items: usize,
    fp_rate: f64,
}

impl ConcurrentBloomFilter {
    /// Create a new concurrent bloom filter
    #[must_use]
    pub fn new(expected_items: usize, fp_rate: f64) -> Self {
        Self {
            filter: RwLock::new(ScalableBloomFilter::with_capacity(expected_items, fp_rate)),
            expected_items,
            fp_rate,
        }
    }

    /// Load bloom filter from file, or create new if not exists/invalid.
    ///
    /// # Errors
    /// Returns error if file cannot be read (but not if it doesn't exist).
    pub fn load_or_create(
        path: &std::path::Path,
        expected_items: usize,
        fp_rate: f64,
    ) -> std::io::Result<Self> {
        if path.exists() {
            let data = std::fs::read(path)?;
            if let Some(filter) = Self::from_bytes(&data, expected_items, fp_rate) {
                return Ok(filter);
            }
        }
        Ok(Self::new(expected_items, fp_rate))
    }

    /// Deserialize from bytes.
    #[must_use]
    pub fn from_bytes(data: &[u8], expected_items: usize, fp_rate: f64) -> Option<Self> {
        if data.len() < 4 || &data[..4] != BLOOM_MAGIC {
            return None;
        }

        let filter = ScalableBloomFilter::from_bytes(&data[4..])?;

        Some(Self {
            filter: RwLock::new(filter),
            expected_items,
            fp_rate,
        })
    }

    /// Serialize to bytes for persistence.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let filter_bytes = self.filter.read().to_bytes();
        let mut result = Vec::with_capacity(BLOOM_MAGIC.len() + filter_bytes.len());
        result.extend_from_slice(BLOOM_MAGIC);
        result.extend(filter_bytes);
        result
    }

    /// Save bloom filter to file atomically.
    ///
    /// # Errors
    /// Returns error if file cannot be written.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;

        let data = self.to_bytes();

        // Write atomically via temp file
        let temp_path = path.with_extension("tmp");

        if let Some(parent) = temp_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(&data)?;
        file.sync_all()?;

        std::fs::rename(&temp_path, path)?;
        Ok(())
    }

    /// Insert an item
    pub fn insert<T: Hash>(&self, item: &T) {
        self.filter.write().insert(item);
    }

    /// Insert bytes directly
    pub fn insert_bytes(&self, item: &[u8]) {
        self.filter.write().add_bytes(item);
    }

    /// Check if item might be present
    #[must_use]
    pub fn may_contain<T: Hash>(&self, item: &T) -> bool {
        self.filter.read().may_contain(item)
    }

    /// Check if bytes might be present
    #[must_use]
    pub fn may_contain_bytes(&self, item: &[u8]) -> bool {
        self.filter.read().exists_bytes(item)
    }

    /// Clear the filter
    pub fn clear(&self) {
        self.filter.write().clear();
    }

    /// Rebuild the filter with new items
    pub fn rebuild<T, I>(&self, items: I)
    where
        T: Hash,
        I: IntoIterator<Item = T>,
    {
        let mut new_filter = ScalableBloomFilter::with_capacity(self.expected_items, self.fp_rate);
        for item in items {
            new_filter.insert(&item);
        }
        *self.filter.write() = new_filter;
    }

    /// Rebuild from byte slices (for cache keys)
    pub fn rebuild_from_bytes<'a, I>(&self, items: I)
    where
        I: IntoIterator<Item = &'a [u8]>,
    {
        let mut new_filter = ScalableBloomFilter::with_capacity(self.expected_items, self.fp_rate);
        for item in items {
            new_filter.add_bytes(item);
        }
        *self.filter.write() = new_filter;
    }

    /// Get the number of items in the filter
    #[must_use]
    pub fn len(&self) -> usize {
        self.filter.read().len()
    }

    /// Check if filter is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filter.read().is_empty()
    }

    /// Get memory usage in bytes
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        self.filter.read().memory_usage()
    }

    /// Get statistics
    #[must_use]
    pub fn stats(&self) -> BloomFilterStats {
        let filter = self.filter.read();
        BloomFilterStats {
            count: filter.len() as u64,
            num_bits: filter.filters.first().map_or(0, BloomFilter::num_bits),
            num_hashes: filter.filters.first().map_or(0, BloomFilter::num_hashes),
            memory_bytes: filter.memory_usage(),
            estimated_fp_rate: filter.error_rate(),
            num_sub_filters: filter.num_filters(),
            capacity: filter.capacity(),
            is_empty: filter.is_empty(),
        }
    }
}

/// Bloom filter statistics
#[derive(Debug, Clone)]
pub struct BloomFilterStats {
    /// Number of items inserted
    pub count: u64,
    /// Configured capacity
    pub capacity: usize,
    /// Whether the filter is empty
    pub is_empty: bool,
    /// Number of bits in the primary filter
    pub num_bits: usize,
    /// Number of hash functions
    pub num_hashes: usize,
    /// Memory usage in bytes
    pub memory_bytes: usize,
    /// Estimated false positive rate
    pub estimated_fp_rate: f64,
    /// Number of sub-filters (for scalable bloom filter)
    pub num_sub_filters: usize,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_murmurhash64a() {
        let hash1 = murmurhash64a(b"hello", 0);
        let hash2 = murmurhash64a(b"hello", 0);
        let hash3 = murmurhash64a(b"world", 0);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_bloom_filter_basic() {
        let mut bf = BloomFilter::new(1000, 0.01);

        assert!(!bf.exists_bytes(b"test"));
        bf.add_bytes(b"test");
        assert!(bf.exists_bytes(b"test"));
        assert!(!bf.exists_bytes(b"not_added"));
    }

    #[test]
    fn test_bloom_filter_hashable() {
        let mut bf = BloomFilter::new(1000, 0.01);

        assert!(!bf.may_contain(&"hello"));
        bf.insert(&"hello");
        assert!(bf.may_contain(&"hello"));
        assert!(!bf.may_contain(&"world"));
    }

    #[test]
    fn test_bloom_filter_false_positive_rate() {
        let mut bf = BloomFilter::new(1000, 0.01);

        // Add items
        for i in 0..1000 {
            bf.add_bytes(format!("item{i}").as_bytes());
        }

        // Check false positives (smaller sample)
        let mut false_positives = 0;
        for i in 1000..2000 {
            if bf.exists_bytes(format!("item{i}").as_bytes()) {
                false_positives += 1;
            }
        }

        let fp_rate = false_positives as f64 / 1000.0;
        // Allow some margin (should be close to 1%)
        assert!(fp_rate < 0.05, "False positive rate too high: {fp_rate}");
    }

    #[test]
    fn test_scalable_bloom_filter() {
        let config = BloomFilterConfig {
            capacity: 50,
            error_rate: 0.01,
            expansion: 2,
            nonscaling: false,
        };
        let mut sbf = ScalableBloomFilter::new(config);

        // Add items past capacity
        for i in 0..150 {
            sbf.add_bytes(format!("item{i}").as_bytes());
        }

        // Should have scaled
        assert!(sbf.num_filters() > 1);

        // All items should exist
        for i in 0..150 {
            assert!(sbf.exists_bytes(format!("item{i}").as_bytes()));
        }
    }

    #[test]
    fn test_serialization() {
        let mut sbf = ScalableBloomFilter::with_capacity(50, 0.01);

        for i in 0..25 {
            sbf.add_bytes(format!("test{i}").as_bytes());
        }

        let bytes = sbf.to_bytes();
        let restored = ScalableBloomFilter::from_bytes(&bytes).unwrap();

        assert_eq!(sbf.len(), restored.len());
        assert_eq!(sbf.num_filters(), restored.num_filters());

        // Check items exist in restored
        for i in 0..25 {
            assert!(restored.exists_bytes(format!("test{i}").as_bytes()));
        }
    }

    #[test]
    fn test_concurrent_bloom_filter() {
        let bf = ConcurrentBloomFilter::new(1000, 0.01);

        bf.insert(&"concurrent");
        assert!(bf.may_contain(&"concurrent"));
        assert!(!bf.may_contain(&"not_there"));

        let stats = bf.stats();
        assert_eq!(stats.count, 1);
    }
}
