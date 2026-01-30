# Libretto

<p align="center">
  <strong>A blazingly fast, Composer-compatible package manager for PHP — written in Rust</strong>
</p>

<p align="center">
  <a href="https://github.com/libretto-pm/libretto/actions"><img src="https://github.com/libretto-pm/libretto/workflows/CI/badge.svg" alt="CI Status"></a>
  <a href="https://github.com/libretto-pm/libretto/blob/master/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg" alt="License"></a>
  <a href="https://github.com/libretto-pm/libretto"><img src="https://img.shields.io/badge/rust-1.89%2B-orange.svg" alt="Rust Version"></a>
</p>

---

## Overview

Libretto is a high-performance drop-in replacement for [Composer](https://getcomposer.org/), the PHP dependency manager. Built from the ground up in Rust, it leverages modern techniques like parallel downloads, SIMD-accelerated operations, and intelligent caching to dramatically speed up your PHP dependency management workflow.

### Key Features

- 🚀 **Blazingly Fast** — Parallel HTTP/2 downloads, SIMD-accelerated JSON parsing, and zero-copy deserialization
- 📦 **Composer Compatible** — Works with your existing `composer.json` and `composer.lock` files
- 🔒 **Secure** — Built-in security auditing, integrity verification, and pure-Rust TLS
- 🌍 **Cross-Platform** — Native binaries for Linux, macOS, and Windows (x86_64 and ARM64)
- 💾 **Smart Caching** — Multi-tier content-addressable cache with zstd compression
- 🧩 **Modern Resolver** — PubGrub-based dependency resolution with clear conflict explanations

## Installation

### Pre-built Binaries

Download the latest release for your platform from the [Releases](https://github.com/libretto-pm/libretto/releases) page.

### Build from Source

Requires Rust 1.89 or later:

```bash
git clone https://github.com/libretto-pm/libretto.git
cd libretto
cargo build --release
```

The binary will be available at `target/release/libretto`.

## Usage

Libretto provides familiar Composer-compatible commands:

```bash
# Install dependencies from composer.json
libretto install

# Update all dependencies
libretto update

# Add a new package
libretto require vendor/package

# Add a dev dependency
libretto require --dev vendor/package

# Remove a package
libretto remove vendor/package

# Search for packages
libretto search "search term"

# Show package information
libretto show vendor/package

# Initialize a new project
libretto init

# Validate composer.json
libretto validate

# Regenerate autoloader
libretto dump-autoload

# Check for security vulnerabilities
libretto audit

# Clear the cache
libretto cache:clear
```

### Global Options

```bash
-v, --verbose       Enable verbose output
-d, --working-dir   Set the working directory
    --no-ansi       Disable ANSI colors
-h, --help          Print help
-V, --version       Print version
```

## Performance

Libretto achieves its performance through several techniques:

| Feature | Technology |
|---------|------------|
| JSON Parsing | `sonic-rs` with SIMD acceleration |
| HTTP Client | `reqwest` with HTTP/2 multiplexing |
| Hashing | BLAKE3 with SIMD (SSE4.2/AVX2/NEON) |
| Caching | Multi-tier with `moka` + zstd compression |
| Parallelism | `tokio` async + `rayon` work-stealing |
| Memory | `mimalloc` allocator + zero-copy with `rkyv` |
| Resolution | PubGrub algorithm (from `uv` project) |

## Architecture

Libretto is organized as a Cargo workspace with modular crates:

```
crates/
├── libretto-cli          # Command-line interface
├── libretto-core         # Core types and utilities
├── libretto-platform     # Cross-platform compatibility layer
├── libretto-cache        # Multi-tier caching system
├── libretto-repository   # Package repository clients
├── libretto-resolver     # PubGrub dependency resolution
├── libretto-downloader   # Parallel package downloading
├── libretto-archive      # ZIP/TAR extraction
├── libretto-vcs          # Git operations
├── libretto-autoloader   # PHP autoloader generation
├── libretto-plugin-system# Composer plugin compatibility
├── libretto-audit        # Security vulnerability checking
└── libretto-lockfile     # Atomic lockfile management
```

## Composer Script Compatibility

Libretto provides **full compatibility** with Composer lifecycle scripts. You don't need to change anything in your projects - scripts work exactly as they do with Composer.

### How It Works

| Script Type | Handling |
|-------------|----------|
| Shell commands | Executed directly (`echo`, `php artisan`, etc.) |
| `@php` directives | Executed via PHP binary |
| `@composer` directives | Executed via Libretto |
| `@putenv` directives | Sets environment variables |
| PHP class callbacks | Full Composer Event API support |

### Full Composer Event API

Composer scripts can reference PHP static methods like `Illuminate\Foundation\ComposerScripts::postAutoloadDump`. These require a `Composer\Script\Event` object with access to:

- `$event->getComposer()` - Composer instance
- `$event->getIO()` - Console I/O interface
- `$event->isDevMode()` - Development mode flag
- `$event->getComposer()->getConfig()->get('vendor-dir')` - Configuration values

**Libretto provides complete compatibility through two mechanisms:**

1. **Real Composer Support**: If your project has `composer/composer` as a dependency, Libretto uses the real Composer classes
2. **Built-in Stubs**: If not, Libretto loads comprehensive stubs that implement the full Composer Event API

This means **any PHP callback that works with Composer will work with Libretto** - no modifications needed.

### Performance Optimization

For maximum speed, well-known callbacks are handled directly in Rust:

| Callback | Optimization |
|----------|-------------|
| `Illuminate\Foundation\ComposerScripts::postAutoloadDump` | Cache files cleared directly in Rust |
| `Illuminate\Foundation\ComposerScripts::postInstall` | Handled natively |
| `Illuminate\Foundation\ComposerScripts::postUpdate` | Handled natively |
| `Composer\Config::disableProcessTimeout` | No-op (Libretto has no timeout) |

### Environment Variables

Scripts can detect Libretto via these environment variables:

```bash
LIBRETTO=1              # Always set when running under Libretto
COMPOSER_DEV_MODE=1     # "1" for dev, "0" for production
COMPOSER_VENDOR_DIR=/path/to/vendor
```

### Supported Frameworks

| Framework | Status | Notes |
|-----------|--------|-------|
| Laravel | Full Support | All scripts work, optimized handling |
| Symfony | Full Support | Full Flex compatibility |
| Drupal | Full Support | Standard Composer scripts |
| Any Other | Full Support | Any standard Composer scripts work |

### Limitations

- **Composer Plugins**: Deep plugin integration (plugins that modify Composer's internal behavior) is not supported. However, script callbacks from plugins work fine.
- **Interactive Prompts**: Scripts that require complex interactive input may have limited functionality in non-TTY environments.

## Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| Linux | x86_64 | ✅ Full Support |
| Linux | aarch64 | ✅ Full Support |
| macOS | x86_64 (Intel) | ✅ Full Support |
| macOS | aarch64 (Apple Silicon) | ✅ Full Support |
| Windows | x86_64 | ✅ Full Support |

### Platform-Specific Optimizations

- **Linux**: io_uring support (5.1+), AVX2/AVX-512 SIMD
- **macOS**: kqueue I/O, NEON SIMD on Apple Silicon
- **Windows**: IOCP I/O, AVX2 SIMD

## Development

### Prerequisites

- Rust 1.89 or later
- For cross-compilation: appropriate target toolchains

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run clippy lints
cargo clippy --all-targets --all-features -- -D warnings

# Format code
cargo fmt --all

# Run benchmarks
cargo bench
```

### Cross-Compilation

Aliases are provided in `.cargo/config.toml`:

```bash
cargo linux-x64     # x86_64-unknown-linux-gnu
cargo linux-arm64   # aarch64-unknown-linux-gnu
cargo macos-x64     # x86_64-apple-darwin
cargo macos-arm64   # aarch64-apple-darwin
cargo windows-x64   # x86_64-pc-windows-msvc
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. For major changes, please open an issue first to discuss what you would like to change.

### Code Style

- Follow Rust conventions and idioms
- Run `cargo fmt` before committing
- Ensure `cargo clippy` passes without warnings
- Add tests for new functionality
- Update documentation as needed

## License

Libretto is dual-licensed under the MIT License and Apache License 2.0. You may choose either license.

- [MIT License](LICENSE-MIT)
- [Apache License 2.0](LICENSE-APACHE)

## Acknowledgments

- [Composer](https://getcomposer.org/) — The original PHP dependency manager
- [uv](https://github.com/astral-sh/uv) — Inspiration for performance techniques and PubGrub implementation
- [Packagist](https://packagist.org/) — The PHP package repository

---

<p align="center">
  Made with ❤️ and 🦀
</p>
