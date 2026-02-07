//! Comprehensive benchmark suite for Libretto.
//!
//! This crate provides benchmarking utilities and fixtures for measuring
//! libretto's performance against Composer.
//!
//! # Benchmark Categories
//!
//! - **Dependency Resolution**: Simple to complex graphs
//! - **Package Operations**: Download, extraction, verification
//! - **Cache Operations**: Cold/warm cache, lookup, eviction
//! - **Autoloader Generation**: Various project sizes
//! - **Lock File Operations**: Parse, generate, diff
//! - **VCS Operations**: Clone, checkout, submodules
//! - **Real-World Scenarios**: Laravel, Symfony simulations
//! - **Memory Benchmarks**: Peak usage, allocations
//! - **SIMD Benchmarks**: Vectorized operations
//!
//! # Running Benchmarks
//!
//! ```bash
//! # Run all benchmarks
//! cargo bench --package libretto-bench
//!
//! # Run specific benchmark group
//! cargo bench --package libretto-bench --bench dependency_resolution
//!
//! # Save baseline for comparison
//! cargo bench --package libretto-bench -- --save-baseline main
//!
//! # Compare against baseline
//! cargo bench --package libretto-bench -- --baseline main
//! ```

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod fixtures;
pub mod generators;

#[cfg(feature = "composer-comparison")]
pub mod composer_runner;

use std::time::{Duration, Instant};

/// Benchmark result with timing and metadata.
#[derive(Debug, Clone)]
pub struct BenchResult {
    /// Name of the benchmark.
    pub name: String,
    /// Duration of the operation.
    pub duration: Duration,
    /// Number of iterations.
    pub iterations: u64,
    /// Optional memory usage in bytes.
    pub memory_bytes: Option<u64>,
}

impl BenchResult {
    /// Create a new benchmark result.
    #[must_use]
    pub fn new(name: impl Into<String>, duration: Duration, iterations: u64) -> Self {
        Self {
            name: name.into(),
            duration,
            iterations,
            memory_bytes: None,
        }
    }

    /// Add memory usage information.
    #[must_use]
    pub fn with_memory(mut self, bytes: u64) -> Self {
        self.memory_bytes = Some(bytes);
        self
    }

    /// Calculate operations per second.
    #[must_use]
    pub fn ops_per_second(&self) -> f64 {
        if self.duration.as_secs_f64() == 0.0 {
            return 0.0;
        }
        self.iterations as f64 / self.duration.as_secs_f64()
    }
}

/// Comparison result between two benchmark runs.
#[derive(Debug, Clone)]
pub struct ComparisonResult {
    /// Libretto timing.
    pub libretto: Duration,
    /// Composer timing (if available).
    pub composer: Option<Duration>,
    /// Speedup factor (`composer_time` / `libretto_time`).
    pub speedup: Option<f64>,
}

impl ComparisonResult {
    /// Create comparison from libretto-only result.
    #[must_use]
    pub fn libretto_only(duration: Duration) -> Self {
        Self {
            libretto: duration,
            composer: None,
            speedup: None,
        }
    }

    /// Create comparison with both tools.
    #[must_use]
    pub fn with_composer(libretto: Duration, composer: Duration) -> Self {
        let speedup = if libretto.as_secs_f64() > 0.0 {
            Some(composer.as_secs_f64() / libretto.as_secs_f64())
        } else {
            None
        };
        Self {
            libretto,
            composer: Some(composer),
            speedup,
        }
    }

    /// Check if libretto is faster than composer.
    #[must_use]
    pub fn is_faster(&self) -> Option<bool> {
        self.speedup.map(|s| s > 1.0)
    }
}

/// Simple timer for manual benchmarking.
#[derive(Debug)]
pub struct Timer {
    start: Instant,
}

impl Timer {
    /// Start a new timer.
    #[must_use]
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Get elapsed duration without stopping.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Stop and return elapsed duration.
    #[must_use]
    pub fn stop(self) -> Duration {
        self.start.elapsed()
    }
}

/// Get peak memory usage on Linux via /proc/self/status.
#[cfg(target_os = "linux")]
pub fn peak_memory_bytes() -> Option<u64> {
    use std::fs;
    let status = fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with("VmPeak:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(kb) = parts[1].parse::<u64>() {
                    return Some(kb * 1024);
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn peak_memory_bytes() -> Option<u64> {
    None
}

/// Get current RSS (Resident Set Size) on Linux.
#[cfg(target_os = "linux")]
pub fn current_rss_bytes() -> Option<u64> {
    use std::fs;
    let status = fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(kb) = parts[1].parse::<u64>() {
                    return Some(kb * 1024);
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn current_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_result() {
        let result = BenchResult::new("test", Duration::from_secs(1), 100);
        assert!((result.ops_per_second() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_comparison_faster() {
        let cmp =
            ComparisonResult::with_composer(Duration::from_millis(100), Duration::from_millis(500));
        assert_eq!(cmp.is_faster(), Some(true));
        assert!((cmp.speedup.unwrap() - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_timer() {
        let timer = Timer::start();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.stop();
        assert!(elapsed >= Duration::from_millis(10));
    }
}
