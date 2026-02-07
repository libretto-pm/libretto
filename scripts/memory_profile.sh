#!/bin/bash
# Memory profiling script for libretto
# Uses dhat-rs for heap profiling

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTPUT_DIR="${PROJECT_ROOT}/target/memory-profile"

mkdir -p "$OUTPUT_DIR"

echo "=== Libretto Memory Profiler ==="
echo ""

# Parse arguments
PROFILE_TYPE="${1:-benchmark}"
BENCHMARK_NAME="${2:-memory_benchmarks}"

case "$PROFILE_TYPE" in
    benchmark|bench)
        echo "Memory profiling benchmark: $BENCHMARK_NAME"
        echo ""
        
        cd "$PROJECT_ROOT"
        
        # Run with dhat feature enabled
        DHAT_SAVE_PATH="$OUTPUT_DIR/dhat_${BENCHMARK_NAME}.json" \
        cargo bench \
            --package libretto-bench \
            --bench "$BENCHMARK_NAME" \
            --features dhat-heap \
            -- --noplot
        
        echo ""
        echo "=== Memory profile saved ==="
        echo "Output: $OUTPUT_DIR/dhat_${BENCHMARK_NAME}.json"
        echo ""
        echo "View with dhat-viewer:"
        echo "  https://nnethercote.github.io/dh_view/dh_view.html"
        ;;
        
    valgrind)
        echo "Valgrind memory profiling"
        echo ""
        
        if ! command -v valgrind &> /dev/null; then
            echo "Error: valgrind not found. Install with:"
            echo "  sudo apt install valgrind"
            exit 1
        fi
        
        # Build release binary
        cd "$PROJECT_ROOT"
        cargo build --release --package libretto-cli
        
        # Create test project
        TEMP_DIR=$(mktemp -d)
        cat > "$TEMP_DIR/composer.json" << 'EOF'
{
    "name": "test/valgrind",
    "require": {}
}
EOF
        
        echo "Running valgrind..."
        valgrind \
            --tool=massif \
            --massif-out-file="$OUTPUT_DIR/massif.out" \
            ./target/release/libretto \
            --project-dir "$TEMP_DIR" \
            validate 2>/dev/null || true
        
        rm -rf "$TEMP_DIR"
        
        echo ""
        echo "=== Massif output saved ==="
        echo "Output: $OUTPUT_DIR/massif.out"
        echo ""
        echo "View with ms_print:"
        echo "  ms_print $OUTPUT_DIR/massif.out"
        ;;
        
    cachegrind)
        echo "Cachegrind profiling"
        echo ""
        
        if ! command -v valgrind &> /dev/null; then
            echo "Error: valgrind not found. Install with:"
            echo "  sudo apt install valgrind"
            exit 1
        fi
        
        # Build release binary
        cd "$PROJECT_ROOT"
        cargo build --release --package libretto-cli
        
        # Create test project
        TEMP_DIR=$(mktemp -d)
        cat > "$TEMP_DIR/composer.json" << 'EOF'
{
    "name": "test/cachegrind",
    "require": {}
}
EOF
        
        echo "Running cachegrind..."
        valgrind \
            --tool=cachegrind \
            --cachegrind-out-file="$OUTPUT_DIR/cachegrind.out" \
            ./target/release/libretto \
            --project-dir "$TEMP_DIR" \
            validate 2>/dev/null || true
        
        rm -rf "$TEMP_DIR"
        
        echo ""
        echo "=== Cachegrind output saved ==="
        echo "Output: $OUTPUT_DIR/cachegrind.out"
        echo ""
        echo "View with cg_annotate:"
        echo "  cg_annotate $OUTPUT_DIR/cachegrind.out"
        ;;
        
    rss)
        echo "RSS memory tracking during benchmark"
        echo ""
        
        cd "$PROJECT_ROOT"
        
        # Build and run with memory tracking
        cargo bench \
            --package libretto-bench \
            --bench memory_benchmarks \
            -- --noplot 2>&1 | tee "$OUTPUT_DIR/rss_output.txt"
        
        echo ""
        echo "=== RSS output saved ==="
        echo "Output: $OUTPUT_DIR/rss_output.txt"
        ;;
        
    *)
        echo "Usage: $0 [benchmark|valgrind|cachegrind|rss] [benchmark_name]"
        echo ""
        echo "Examples:"
        echo "  $0 benchmark memory_benchmarks"
        echo "  $0 valgrind"
        echo "  $0 cachegrind"
        echo "  $0 rss"
        exit 1
        ;;
esac
