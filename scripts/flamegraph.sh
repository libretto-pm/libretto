#!/bin/bash
# Flamegraph generation script for libretto
# Requires: cargo-flamegraph (cargo install flamegraph)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTPUT_DIR="${PROJECT_ROOT}/target/flamegraph"

mkdir -p "$OUTPUT_DIR"

echo "=== Libretto Flamegraph Generator ==="
echo ""

# Check if cargo-flamegraph is installed
if ! command -v cargo flamegraph &> /dev/null; then
    echo "Error: cargo-flamegraph not found. Install with:"
    echo "  cargo install flamegraph"
    exit 1
fi

# Check for perf on Linux
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    if ! command -v perf &> /dev/null; then
        echo "Warning: perf not found. Install with:"
        echo "  sudo apt install linux-tools-common linux-tools-generic"
    fi
fi

# Parse arguments
PROFILE_TYPE="${1:-benchmark}"
BENCHMARK_NAME="${2:-lockfile_operations}"

case "$PROFILE_TYPE" in
    benchmark|bench)
        echo "Profiling benchmark: $BENCHMARK_NAME"
        echo "Output: $OUTPUT_DIR/${BENCHMARK_NAME}_flamegraph.svg"
        echo ""
        
        cd "$PROJECT_ROOT"
        cargo flamegraph \
            --package libretto-bench \
            --bench "$BENCHMARK_NAME" \
            --output "$OUTPUT_DIR/${BENCHMARK_NAME}_flamegraph.svg" \
            -- --bench --profile-time 10
        ;;
        
    cli)
        echo "Profiling CLI install command"
        echo "Output: $OUTPUT_DIR/cli_install_flamegraph.svg"
        echo ""
        
        # Create a temporary test project
        TEMP_DIR=$(mktemp -d)
        cat > "$TEMP_DIR/composer.json" << 'EOF'
{
    "name": "test/flamegraph",
    "require": {
        "symfony/console": "^6.0"
    }
}
EOF
        
        cd "$PROJECT_ROOT"
        cargo flamegraph \
            --package libretto-cli \
            --bin libretto \
            --output "$OUTPUT_DIR/cli_install_flamegraph.svg" \
            -- --project-dir "$TEMP_DIR" install --no-scripts 2>/dev/null || true
        
        rm -rf "$TEMP_DIR"
        ;;
        
    *)
        echo "Usage: $0 [benchmark|cli] [benchmark_name]"
        echo ""
        echo "Examples:"
        echo "  $0 benchmark lockfile_operations"
        echo "  $0 benchmark dependency_resolution"
        echo "  $0 cli"
        exit 1
        ;;
esac

echo ""
echo "=== Flamegraph generated! ==="
echo "View: $OUTPUT_DIR"
