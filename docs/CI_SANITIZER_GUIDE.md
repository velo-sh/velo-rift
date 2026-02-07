# CI Sanitizer Integration Guide

## Thread Sanitizer (TSan)

Detects data races, deadlocks, and thread safety issues in the inception layer.

### Build & Run

```bash
# Requires nightly toolchain
rustup toolchain install nightly

# Build with TSan
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test \
  --target aarch64-apple-darwin \
  -p vrift-inception-layer \
  -- --test-threads=1

# Run specific stress tests under TSan
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly build \
  --target aarch64-apple-darwin --release -p vrift-inception-layer
TSAN_OPTIONS="report_bugs=1:halt_on_error=0" \
  bash tests/qa_v2/repro_rwlock_stress.sh
```

### Known Suppressions

TSan may report false positives on:
- `FlightRecorder` lock-free buffer (benign relaxed ordering races)
- `INCEPTION_LAYER_STATE` double-checked locking (protected by SeqCst CAS)
- `DirtyTracker` linear probing (benign: hash-based slot races resolve naturally)

Create a `tsan.suppressions` file:
```
race:FlightRecorder
race:DirtyTracker
```

Run with: `TSAN_OPTIONS="suppressions=tsan.suppressions" ...`

## Address Sanitizer (ASan)

Detects use-after-free, buffer overflows, and memory leaks.

```bash
RUSTFLAGS="-Z sanitizer=address" cargo +nightly test \
  --target aarch64-apple-darwin \
  -p vrift-inception-layer
```

> **WARNING**: ASan conflicts with DYLD_INSERT_LIBRARIES on macOS.
> ASan tests should run unit tests only, not E2E shim injection tests.

## GitHub Actions CI Example

```yaml
sanitizer-tests:
  runs-on: macos-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@nightly
      with:
        targets: aarch64-apple-darwin
    - name: TSan unit tests
      run: |
        RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test \
          --target aarch64-apple-darwin \
          -p vrift-inception-layer \
          -- --test-threads=1
      env:
        TSAN_OPTIONS: "report_bugs=1:halt_on_error=0"
```

## Local Quick Check

```bash
# Run full QA suite (stable toolchain, no sanitizers)
bash tests/qa_v2/run_all_qa.sh

# Run with stack frame guard (CI-critical)
bash tests/qa_v2/check_stack_frame.sh

# Run QA filtered to boot safety + stress
bash tests/qa_v2/run_all_qa.sh --filter=boot --stress
```
