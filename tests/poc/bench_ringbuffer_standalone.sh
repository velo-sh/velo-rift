#!/bin/bash
# Quick standalone benchmark — MPSC channel throughput
# Compares std::sync::mpsc with crossbeam-style bounded channel.

TMPDIR="${TMPDIR:-/tmp}"
BENCH_SRC="$TMPDIR/bench_rb.rs"
BENCH_BIN="$TMPDIR/bench_rb"

cat > "$BENCH_SRC" << 'EOF'
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::time::Instant;

fn main() {
    const ITERATIONS: usize = 5_000_000;
    const PRODUCERS: usize = 4;
    let per_producer = ITERATIONS / PRODUCERS;

    println!("=== MPSC Channel Benchmark ===");
    println!("Iterations: {}, Producers: {}\n", ITERATIONS, PRODUCERS);

    let (tx, rx) = mpsc::sync_channel::<usize>(4096);
    let barrier = Arc::new(Barrier::new(PRODUCERS + 1));

    // Spawn producers
    let mut handles = vec![];
    for _ in 0..PRODUCERS {
        let tx = tx.clone();
        let bar = barrier.clone();
        handles.push(std::thread::spawn(move || {
            bar.wait();
            for i in 0..per_producer {
                tx.send(i).unwrap();
            }
        }));
    }
    drop(tx); // Close sender so receiver knows when done

    // Consumer in main thread
    barrier.wait();
    let start = Instant::now();

    let mut consumed: usize = 0;
    for _ in rx {
        consumed += 1;
    }

    let elapsed = start.elapsed();

    // Wait for producers
    for h in handles {
        h.join().unwrap();
    }

    println!("Time: {:.3}s", elapsed.as_secs_f64());
    println!("Throughput: {:.2} M ops/s", consumed as f64 / elapsed.as_secs_f64() / 1_000_000.0);
    println!("Avg latency: {:.1} ns/op", elapsed.as_nanos() as f64 / consumed as f64);
    println!("Consumed: {}", consumed);
    assert_eq!(consumed, ITERATIONS, "Lost messages!");
}
EOF

rustc -O --edition 2021 "$BENCH_SRC" -o "$BENCH_BIN" && "$BENCH_BIN"
