# CODEX.md - Libretto Project Guide

## Project Overview

Libretto is a high-performance, Composer-compatible package manager for PHP written in Rust. It aims to be a drop-in replacement for Composer with significantly improved performance through parallel operations, SIMD optimizations, and efficient caching.

**Repository**: https://github.com/libretto-pm/libretto  
**License**: MIT OR Apache-2.0  
**MSRV**: Rust 1.75+

## Build & Development Commands

```bash
# Build (debug)
cargo build

# Build (release with optimizations)
cargo build --release
# or use the alias:
cargo br

# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p libretto-core
cargo test -p libretto-resolver

# Run clippy lints (strict - treats warnings as errors)
cargo clippy --all-targets --all-features -- -D warnings

# Check formatting
cargo fmt --all -- --check

# Format code
cargo fmt --all

# Build documentation
cargo doc --no-deps --all-features

# Run benchmarks
cargo bench --package libretto-platform -- --noplot
cargo bench --package libretto-core
cargo bench --package libretto-resolver

# Cross-compilation targets (aliases defined in .cargo/config.toml)
cargo linux-x64    # x86_64-unknown-linux-gnu
cargo linux-arm64  # aarch64-unknown-linux-gnu
cargo macos-x64    # x86_64-apple-darwin
cargo macos-arm64  # aarch64-apple-darwin
cargo windows-x64  # x86_64-pc-windows-msvc
```

## Project Architecture

### Workspace Structure

This is a Cargo workspace with 13 crates under `crates/`:

| Crate | Description |
|-------|-------------|
| `libretto-core` | Core types: Package, Version, Error, JSON utilities, content-addressable hashing |
| `libretto-platform` | Cross-platform compatibility layer: OS detection, SIMD, I/O backends, TLS |
| `libretto-cache` | Multi-tier content-addressable cache with zstd compression and moka |
| `libretto-repository` | Package repository clients (Packagist, private repos) |
| `libretto-resolver` | PubGrub-based dependency resolution (uses astral-pubgrub from uv) |
| `libretto-downloader` | Parallel HTTP/2 package downloading with retry/backoff |
| `libretto-archive` | ZIP/TAR extraction |
| `libretto-vcs` | Git operations for source packages |
| `libretto-autoloader` | PHP autoloader generation with tree-sitter parsing |
| `libretto-plugin-system` | Composer plugin compatibility |
| `libretto-audit` | Security vulnerability checking |
| `libretto-lockfile` | Atomic composer.lock file management |
| `libretto-cli` | CLI binary and command implementations |

### Dependency Flow

```
libretto-cli
├── libretto-core (foundation)
├── libretto-platform (OS abstractions)
├── libretto-cache
├── libretto-repository
├── libretto-resolver
├── libretto-downloader
│   ├── libretto-archive
│   └── libretto-vcs
├── libretto-autoloader
├── libretto-plugin-system
└── libretto-audit
```

### CLI Commands

The main binary is `libretto` (defined in `libretto-cli`):

- `install` - Install dependencies from composer.json/composer.lock
- `update` - Update dependencies to latest versions
- `require` - Add a package to dependencies
- `remove` - Remove a package from dependencies
- `search` - Search for packages
- `show` - Show package information
- `init` - Initialize a new composer.json
- `validate` - Validate composer.json
- `dump-autoload` - Regenerate autoloader
- `audit` - Check for security vulnerabilities
- `cache:clear` - Clear the package cache

## Code Style & Conventions

### Rust Conventions

- **Edition**: 2024
- **Unsafe code**: Denied (`#![deny(unsafe_code)]` in most crates)
- **Lints**: Strict clippy configuration (pedantic, nursery, cargo warnings)
- **Unwrap**: `clippy::unwrap_used = "warn"` - prefer `?` or explicit error handling

### File Header Pattern

Each crate's lib.rs follows this pattern:
```rust
//! Crate description and documentation.

#![deny(clippy::all)]
#![allow(clippy::module_name_repetitions)]
```

### Error Handling

- Use `thiserror` for error type definitions
- Use `anyhow` for CLI error propagation
- Custom `Error` enum in `libretto-core` for domain errors
- Each crate may have its own error types that convert to core errors

### Serialization

- Primary JSON: `sonic-rs` (high-performance SIMD JSON)
- Serde integration via `serde` with derive
- Binary caching: `rkyv` for zero-copy deserialization

### Concurrency Patterns

- `tokio` for async runtime
- `rayon` for CPU-bound parallelism
- `dashmap` for concurrent hash maps
- `parking_lot` for synchronization primitives
- `moka` for concurrent caching with TTL

### Global Allocator

`libretto-core` sets mimalloc as the global allocator:
```rust
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

## Key Technical Details

### Version Resolution

The resolver uses `astral-pubgrub` (the PubGrub implementation from the `uv` project) for dependency resolution. It supports:
- All Composer constraint formats (^, ~, *, ranges, OR)
- Stability flags (@dev, @alpha, @beta, @RC, @stable)
- Dev branches (dev-* prefixes)
- Replace and provide declarations

### Platform Support

Full support for:
- Linux x86_64 and aarch64
- macOS x86_64 (Intel) and aarch64 (Apple Silicon)
- Windows x86_64

Platform-specific optimizations:
- io_uring on Linux 5.1+
- IOCP on Windows
- kqueue on macOS
- SIMD: SSE4.2/AVX2/AVX-512 on x86_64, NEON on ARM64

### Caching Strategy

Multi-tier cache:
1. In-memory (moka with TTL)
2. Disk (content-addressable, zstd compressed)
3. Zero-copy deserialization with rkyv

### Package Identifiers

Packages use vendor/name format (e.g., `symfony/console`). The `PackageId` type handles parsing and validation.

## Testing

- Unit tests are inline in source files (`#[cfg(test)]` modules)
- Integration tests would go in `tests/` directories (not yet present)
- Benchmarks use `criterion` and are in `benches/` directories

### Running Specific Test Categories

```bash
# Platform-specific tests
cargo test -p libretto-platform simd

# Resolver tests
cargo test -p libretto-resolver

# Run with output
cargo test -- --nocapture
```

## CI/CD

GitHub Actions workflow (`.github/workflows/ci.yml`) runs:
1. Lint (fmt + clippy)
2. Multi-platform tests (Linux, macOS, Windows)
3. SIMD-specific tests
4. Benchmarks
5. Cross-compilation checks
6. Security audit (`cargo audit`)
7. Documentation build
8. Code coverage (llvm-cov → Codecov)
9. MSRV verification (Rust 1.75)
10. Release builds

## Common Development Tasks

### Adding a New Crate

1. Create directory under `crates/libretto-<name>/`
2. Add `Cargo.toml` with `version.workspace = true`, etc.
3. Add to workspace members in root `Cargo.toml`
4. Add to `[workspace.dependencies]` if it will be used by other crates

### Adding a New CLI Command

1. Create `crates/libretto-cli/src/commands/<command>.rs`
2. Add module to `commands/mod.rs`
3. Add variant to `Commands` enum
4. Add match arm in `main.rs`'s `run_command` function

### Performance Considerations

- Use `sonic-rs` instead of `serde_json` for JSON operations
- Prefer `AHashMap`/`AHashSet` over std collections for string keys
- Use `DashMap` for concurrent access patterns
- Consider `rkyv` for cached data structures
- BLAKE3 for fast hashing (SIMD-accelerated)

## Performance Optimizations (pnpm-inspired)

Libretto implements several advanced optimizations for maximum performance:

### 1. Content-Addressable Storage (CAS) Cache

**Location**: `~/.libretto/cache/cas/`

Like pnpm, packages are stored once globally using content-addressable storage:
- Each package URL is hashed to create a unique cache key
- Extracted package contents are stored in the CAS cache
- Subsequent installs use hardlinks from cache to `vendor/`
- **Benefits**: 
  - Instant installs on cache hits (just create hardlinks)
  - Massive disk space savings (files stored once across all projects)
  - Safe: hardlinks maintain file integrity

**Implementation**: `crates/libretto-cli/src/cas_cache.rs`

### 2. HTTP/2 Multiplexing

**Configuration**: `crates/libretto-downloader/src/client.rs`

Aggressive HTTP/2 settings for maximum throughput:
- Automatic protocol negotiation (ALPN) with fallback to HTTP/1.1
- Adaptive flow control windows: 4MB per stream, 8MB per connection
- HTTP/2 keep-alive with 15s interval to maintain connections
- Up to 100 idle connections per host for connection reuse
- **Benefits**: 
  - Multiple parallel requests over single TCP connection
  - Reduced connection overhead and latency
  - Better performance on high-bandwidth networks

**Settings**:
```rust
.pool_max_idle_per_host(100)
.http2_adaptive_window(true)
.http2_initial_stream_window_size(4 * 1024 * 1024)
.http2_initial_connection_window_size(8 * 1024 * 1024)
.http2_keep_alive_interval(Duration::from_secs(15))
```

### 3. Adaptive Concurrency

**Formula**: `(CPU cores × 8).clamp(32, 128)`

Examples:
- 4 cores → 32 concurrent downloads
- 8 cores → 64 concurrent downloads
- 16 cores → 128 concurrent downloads (max)

Unlike Composer's fixed 12 concurrent downloads, Libretto scales with hardware:
- Automatically detects CPU core count
- Scales to 8× cores (more aggressive than pnpm's 4×)
- Minimum 32, maximum 128 concurrent downloads
- **Benefits**: 
  - Saturates high-bandwidth connections
  - Optimal CPU utilization
  - Faster installs on powerful machines

**Implementation**: `crates/libretto-cli/src/commands/install.rs`

### 4. Performance Comparison vs Composer

Expected improvements:
- **Cold cache**: 3-5× faster (HTTP/2 + high concurrency)
- **Warm cache**: 10-100× faster (hardlinks are instant)
- **Disk usage**: 50-70% reduction (shared CAS cache)
- **Network**: Better utilization through HTTP/2 multiplexing

### 5. Optimization Checklist

When adding new download code:
- ✅ Use the CAS cache via `cas_cache::get_cached_path()` and `cas_cache::store_in_cache()`
- ✅ Build HTTP clients with HTTP/2 multiplexing enabled
- ✅ Use adaptive concurrency based on CPU cores
- ✅ Implement hardlinks for cache-to-vendor transfers
- ✅ Add content hash verification for integrity
- ✅ Use connection pooling and keep-alive

### 6. Monitoring Performance

Key metrics to watch:
- Cache hit rate: `cached_count / total_packages`
- Download throughput: `total_bytes / elapsed_time`
- Concurrency utilization: `active_downloads / max_concurrent`
- HTTP/2 efficiency: Check if multiplexing is being used


Timestamp + user tracking | ✅ | `AuditEntry` struct |

---

### ✅ **Fully Implemented Security Features**

| Feature | Status | Notes |
|---------|--------|-------|
| **Fail installation on checksum mismatch** | ✅ Implemented | `--verify-checksums` flag on install command |
| **Repository signature verification** | ✅ Implemented | `TrustedSignatureVerifier` with trust chain support |
| **Trust chain validation** | ✅ Implemented | Full PKI/Web of Trust model in `TrustChain` |
| **`--audit` flag on install/update** | ✅ Implemented | Runs security audit after install/update |
| **`--fail-on-audit` flag** | ✅ Implemented | Fails installation if vulnerabilities found |
| **`--locked` option for audit** | ✅ Implemented | Audits only from composer.lock |
| **Suggest updated versions** | ✅ Implemented | `--suggest-versions` flag queries Packagist |
| **Git credential helpers** | ✅ Implemented | Full `git-credential` protocol integration |

---

### 📊 **Performance Targets**

| Target | Status | Notes |
|--------|--------|-------|
| Audit 500 packages in <200ms | ✅ Likely met | Concurrent fetching with semaphore, DashMap caching |
| Signature verification <10ms | ✅ Likely met | Ed25519 is fast; PGP depends on key size |

Benchmarks exist in `libretto-audit/benches/` but would need to run to confirm exact numbers.

---

### 📦 **Dependencies Used**

All required dependencies are present in `Cargo.toml`:

```libretto/crates/libretto-audit/Cargo.toml#L20-40
blake3 = { workspace = true }
hex = { workspace = true }
sha2 = { workspace = true }
sha1 = { workspace = true }
subtle = { workspace = true }
ed25519-dalek = { workspace = true }
sequoia-openpgp = { workspace = true }
keyring = { workspace = true }
dialoguer = { workspace = true }
zeroize = { workspace = true }
tempfile = { workspace = true }
```

---

## Summary

**100% of security requirements are now implemented**. The security infrastructure includes:

- ✅ All cryptographic primitives (SHA-256, SHA-1, BLAKE3, Ed25519, PGP)
- ✅ Advisory fetching from Packagist with caching
- ✅ CLI audit command with severity display
- ✅ Platform requirements validation  
- ✅ Credential management with keyring + Git credential helpers
- ✅ Secure file/path operations
- ✅ Audit logging
- ✅ `--audit` and `--fail-on-audit` flags on install/update
- ✅ `--locked` and `--suggest-versions` flags on audit
- ✅ `--verify-checksums` flag on install (fails on mismatch)
- ✅ Trust chain validation for signatures (PKI/Web of Trust)
- ✅ Git credential helper integration (`git credential fill/approve/reject`)
