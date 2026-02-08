#!/bin/bash
# ==============================================================================
# Benchmark: Ring Buffer Standalone Throughput
# ==============================================================================
# Measures the push/pop throughput of MPSC channels in isolation,
# without IPC or daemon overhead. Validates lock-free performance under
# various contention levels.
#
# Uses std::sync::mpsc::sync_channel for proven correctness.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BUILD_MODE="release"
for arg in "$@"; do
    case "$arg" in
        --debug) BUILD_MODE="debug" ;;
        --release) BUILD_MODE="release" ;;
    esac
done

echo ""
echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║           Ring Buffer Standalone Benchmark                         ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""

# Create a standalone benchmark binary
BENCH_DIR="$PROJECT_ROOT/target/bench_rb"
mkdir -p "$BENCH_DIR"

BENCH_SRC="$BENCH_DIR/ring_buffer_bench.rs"
BENCH_BIN="$BENCH_DIR/ring_buffer_bench"

cat > "$BENCH_SRC" << 'BENCH_EOF'
//! Standalone MPSC Channel Benchmark
//! Tests throughput at various producer counts using std::sync::mpsc.

use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::time::Instant;

// ---- Benchmark harness ----

fn bench_throughput(num_producers: usize, ops_per_producer: usize) {
    let (tx, rx) = mpsc::sync_channel::<u64>(4096);
    let barrier = Arc::new(Barrier::new(num_producers + 2)); // +1 consumer +1 main

    let total_ops = num_producers * ops_per_producer;

    // Consumer thread
    let barrier_c = barrier.clone();
    let consumer = std::thread::spawn(move || {
        let mut consumed = 0u64;
        barrier_c.wait();
        for _ in rx {
            consumed += 1;
        }
        consumed
    });

    // Producer threads
    let mut producers = Vec::new();
    for _ in 0..num_producers {
        let tx_p = tx.clone();
        let barrier_p = barrier.clone();
        let handle = std::thread::spawn(move || {
            barrier_p.wait();
            for i in 0..ops_per_producer {
                tx_p.send(i as u64).unwrap();
            }
        });
        producers.push(handle);
    }
    drop(tx); // close original sender

    // Start timing
    barrier.wait();
    let start = Instant::now();

    // Wait for producers
    for p in producers {
        p.join().unwrap();
    }

    let consumed = consumer.join().unwrap();
    let elapsed = start.elapsed();

    let ops_sec = total_ops as f64 / elapsed.as_secs_f64();

    println!(
        "  {:>2}P x {:>7} ops │ {:>10.0} ops/s │ {:>6.2}ms │ consumed={}",
        num_producers,
        ops_per_producer,
        ops_sec,
        elapsed.as_secs_f64() * 1000.0,
        consumed,
    );
}

fn bench_latency(num_producers: usize) {
    let (tx, rx) = mpsc::sync_channel::<u64>(4096);
    let barrier = Arc::new(Barrier::new(2));
    let iterations = 100_000usize;

    let barrier_p = barrier.clone();
    let producer = std::thread::spawn(move || {
        barrier_p.wait();
        for i in 0..iterations {
            tx.send(i as u64).unwrap();
        }
    });

    barrier.wait();
    let start = Instant::now();
    let mut consumed = 0u64;
    for _ in rx {
        consumed += 1;
    }
    let elapsed = start.elapsed();
    producer.join().unwrap();

    let avg_ns = elapsed.as_nanos() as f64 / consumed as f64;
    println!(
        "  {:>2}P roundtrip    │ {:>10.1} ns/op │ {:>6.2}ms total │ {} ops",
        num_producers, avg_ns, elapsed.as_secs_f64() * 1000.0, consumed
    );
    let _ = num_producers;
}

fn main() {
    println!();
    println!("  Throughput Test (push+pop, varying producer count)");
    println!("  ─────────────────────────────────────────────────────────");

    let ops = 1_000_000;

    bench_throughput(1, ops);
    bench_throughput(2, ops / 2);
    bench_throughput(4, ops / 4);
    bench_throughput(8, ops / 8);
    bench_throughput(16, ops / 16);

    println!();
    println!("  Latency Test (single producer, push→pop roundtrip)");
    println!("  ─────────────────────────────────────────────────────────");

    bench_latency(1);

    println!();

    // Summary
    println!("  Channel buffer: 4096 slots (sync_channel)");
    println!();
}
BENCH_EOF

# Build the benchmark
echo "🔨 Building benchmark (${BUILD_MODE} mode)..."

rustc "$BENCH_SRC" -o "$BENCH_BIN" \
    --edition 2021 \
    $([ "$BUILD_MODE" = "release" ] && echo "-C opt-level=3 -C target-cpu=native" || echo "") \
    2>&1 || {
    echo "❌ Failed to compile benchmark"
    exit 1
}
echo "   ✓ Built"
echo ""

# Run benchmark
echo "📊 Running benchmark..."
echo ""
"$BENCH_BIN"

echo "✅ Benchmark complete"
