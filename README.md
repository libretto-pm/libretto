# Libretto

<p align="center">
  <strong>A fast PHP package manager written in Rust</strong>
</p>

<p align="center">
  <a href="https://github.com/libretto-pm/libretto/actions"><img src="https://github.com/libretto-pm/libretto/workflows/CI/badge.svg" alt="CI Status"></a>
  <a href="https://github.com/libretto-pm/libretto/blob/master/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg" alt="License"></a>
  <a href="https://github.com/libretto-pm/libretto"><img src="https://img.shields.io/badge/rust-1.89%2B-orange.svg" alt="Rust Version"></a>
</p>

> **Status: Alpha** - This project is experimental. Use in production at your own risk.

---

## What is Libretto?

Libretto is a PHP package manager written in Rust that can read `composer.json` and `composer.lock` files. It aims to provide faster dependency installation through parallel downloads, content-addressable caching, and native performance.

### What Libretto Is

- A fast alternative for installing PHP dependencies
- Compatible with Composer's file formats (`composer.json`, `composer.lock`)
- Useful for CI pipelines, Docker builds, and scenarios where install speed matters
- A native binary with no PHP runtime required for the install step

### What Libretto Is NOT

- **Not a drop-in replacement for Composer** - Composer plugins are PHP code and cannot run natively in Rust
- **Not feature-complete** - Many Composer features are missing or simplified
- **Not production-ready** - This is alpha software

## Why Libretto?

| Use Case | Benefit |
|----------|---------|
| CI/CD pipelines | Faster builds, no PHP needed for install step |
| Docker builds | Smaller images (no Composer), faster layer caching |
| Monorepos | Parallel downloads scale better with large dependency trees |
| Development | Content-addressable cache means instant installs on cache hit |

### Honest Assessment

The [mago](https://github.com/carthage-software/mago) author makes a fair point: Composer runs maybe 1-5 times per day locally. Saving 2 seconds per run isn't life-changing compared to tools like static analyzers or formatters that run 100+ times daily.

Where Libretto provides real value:
- **CI pipelines** - Dozens of builds per day, each running `install`
- **Cold starts** - Docker builds, new developer onboarding, ephemeral environments
- **Large projects** - More dependencies = more benefit from parallelism

## Installation

### Pre-built Binaries

Download from the [Releases](https://github.com/libretto-pm/libretto/releases) page.

### Build from Source

Requires Rust 1.89 or later:

```bash
git clone https://github.com/libretto-pm/libretto.git
cd libretto
cargo build --release
# Binary at target/release/libretto
```

## Usage

```bash
# Install dependencies
libretto install

# Update dependencies
libretto update

# Add a package
libretto require vendor/package

# Remove a package
libretto remove vendor/package

# Security audit
libretto audit

# Regenerate autoloader
libretto dump-autoload

# Other commands
libretto search "term"
libretto show vendor/package
libretto validate
libretto init
libretto cache:clear
```

## How It Works

### Performance Techniques

| Feature | Implementation |
|---------|----------------|
| JSON parsing | `sonic-rs` with SIMD (SSE4.2/AVX2/NEON) |
| HTTP | HTTP/2 multiplexing, adaptive concurrency |
| Hashing | BLAKE3 with SIMD acceleration |
| Caching | Content-addressable storage with hardlinks |
| Resolution | PubGrub algorithm (from `uv` project) |
| Autoloader | mago-syntax for fast, accurate PHP parsing (~7x faster than tree-sitter) |

### Content-Addressable Cache

Like pnpm, Libretto stores packages once globally:

```
~/.libretto/cache/cas/
├── ab/cdef1234...  # Package contents by hash
├── 12/3456abcd...
└── ...
```

On cache hit, installation is just creating hardlinks - essentially instant.

### Autoloader Generation

Libretto uses [mago-syntax](https://github.com/carthage-software/mago) to scan PHP files for classes, interfaces, traits, and enums. This provides:

- **~7x faster** parsing than tree-sitter-php
- Parallel file scanning with rayon
- Incremental updates via mtime + content hash tracking
- PSR-4, PSR-0, classmap, and files autoloading

## Limitations

### No Plugin Support

Composer plugins are PHP code that hooks into Composer's runtime. This is architecturally impossible to support from native Rust without embedding a PHP interpreter.

If your project relies on plugins like:
- `composer/installers` (custom install paths)
- `phpstan/extension-installer`
- Framework-specific plugins

You should continue using Composer.

### Script Support

| Script Type | Support |
|-------------|---------|
| Shell commands | Works |
| `@php` scripts | Works (calls PHP binary) |
| `@composer` scripts | Partial |
| PHP class callbacks | Basic support via stubs |

Complex callbacks that deeply integrate with Composer's internals may not work correctly.

### Other Limitations

- Private repository authentication is basic
- Some `composer.json` options are ignored
- Error messages are less polished than Composer's
- No interactive prompts for conflicts

## Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| Linux | x86_64, aarch64 | Supported |
| macOS | x86_64, aarch64 | Supported |
| Windows | x86_64 | Supported |

## Architecture

```
crates/
├── libretto-cli          # Command-line interface
├── libretto-core         # Core types, error handling
├── libretto-platform     # OS abstraction layer
├── libretto-cache        # Content-addressable cache
├── libretto-repository   # Packagist client
├── libretto-resolver     # PubGrub dependency resolver
├── libretto-downloader   # Parallel HTTP downloads
├── libretto-archive      # ZIP/TAR extraction
├── libretto-vcs          # Git operations
├── libretto-autoloader   # PHP autoloader generation
├── libretto-audit        # Security vulnerability checks
└── libretto-lockfile     # Lock file management
```

## Development

```bash
# Build
cargo build --release

# Test
cargo test

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# Format
cargo fmt --all

# Benchmark
cargo bench --package libretto-bench
```

## Contributing

Contributions welcome! Please open an issue first for major changes.

## License

Dual-licensed under MIT and Apache 2.0.

## Acknowledgments

- [Composer](https://getcomposer.org/) - The PHP package manager
- [mago](https://github.com/carthage-software/mago) - Fast PHP parser used for autoloader generation
- [uv](https://github.com/astral-sh/uv) - Inspiration for performance techniques
- [pnpm](https://pnpm.io/) - Content-addressable storage inspiration
- [Packagist](https://packagist.org/) - PHP package repository
