#!/bin/bash
# ==============================================================================
# Solid Mode: Cargo Build E2E (User Perspective) — Comprehensive
# ==============================================================================
# Tests the FULL user workflow with behavioral verification:
#
#   Phase 0:  Setup — Create a real Rust project (lib + bin + tests)
#   Phase 1:  Baseline — cargo build + cargo test (no inception)
#   Phase 2:  Ingest — CAS + VDir generation
#   Phase 3:  Inception build — initial build under inception
#   Phase 4:  Real code change — modify function → verify new output
#   Phase 5:  Compile error → fix → rebuild
#   Phase 6:  cargo check + cargo run
#   Phase 7:  Add new module — create module + import → verify
#   Phase 8:  build.rs — build script generates code
#   Phase 9:  Delete source file (refactor)
#   Phase 10: Add dependency (Cargo.toml change)
#   Phase 11: Revert change (undo modification)
#   Phase 12: cargo test — tests pass under inception
#   Phase 13: Clean rebuild — rm -rf target → materialize from CAS
#   Phase 14: Post-inception — exit inception → build still works
#   Phase 15: CAS integrity — hash verification + uchg flags
#   Phase 16: Benchmark timing
#
# Every step verifies BEHAVIORAL CORRECTNESS, not just exit codes.
# ==============================================================================

set -euo pipefail
export RUST_BACKTRACE=1

# ============================================================================
# Configuration
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_WORKSPACE_BASE="/tmp/vdir_cargo_e2e_$$"
SKIP_AUTO_SETUP=1
source "$SCRIPT_DIR/test_setup.sh"

PASSED=0
FAILED=0

# ============================================================================
# Helpers
# ============================================================================
pass() {
    echo "  ✅ PASS: $1"
    PASSED=$((PASSED + 1))
}

fail() {
    echo "  ❌ FAIL: $1"
    FAILED=$((FAILED + 1))
}

assert_output() {
    local desc="$1"; local expected="$2"; shift 2
    local actual
    actual=$("$@" 2>/dev/null) || true
    if echo "$actual" | grep -q "$expected"; then
        pass "$desc"
    else
        fail "$desc (expected '$expected', got '$(echo "$actual" | head -1)')"
    fi
}

assert_no_output() {
    local desc="$1"; local unexpected="$2"; shift 2
    local actual
    actual=$("$@" 2>/dev/null) || true
    if echo "$actual" | grep -q "$unexpected"; then
        fail "$desc (should NOT contain '$unexpected')"
    else
        pass "$desc"
    fi
}

assert_real_file() {
    local path="$1"; local desc="${2:-File is real}"
    if [ -f "$path" ] && [ ! -L "$path" ]; then
        pass "$desc: $(basename "$path")"
    else
        fail "$desc: $path (missing or symlink)"
    fi
}

assert_no_uchg() {
    local path="$1"; local desc="${2:-No uchg flag}"
    if [ "$(uname)" != "Darwin" ]; then pass "$desc (not macOS)"; return; fi
    if ls -lO "$path" 2>/dev/null | grep -q "uchg"; then
        fail "$desc: has uchg!"
    else
        pass "$desc"
    fi
}

assert_recompiled() {
    local build_output="$1"; local crate_name="$2"; local desc="${3:-Recompiled}"
    if echo "$build_output" | grep -q "Compiling $crate_name"; then
        pass "$desc: $crate_name recompiled"
    else
        fail "$desc: $crate_name NOT recompiled"
    fi
}

assert_not_recompiled() {
    local build_output="$1"; local crate_name="$2"; local desc="${3:-Cache hit}"
    if echo "$build_output" | grep -q "Compiling $crate_name"; then
        fail "$desc: $crate_name recompiled (expected cache)"
    else
        pass "$desc: $crate_name cached"
    fi
}

# Run under inception with VDir
INCEP() {
    env \
        VRIFT_PROJECT_ROOT="$TEST_WORKSPACE" \
        VRIFT_VFS_PREFIX="$TEST_WORKSPACE" \
        VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" \
        VR_THE_SOURCE="$VR_THE_SOURCE" \
        VRIFT_VDIR_MMAP="$VDIR_MMAP_PATH" \
        VRIFT_INCEPTION=1 \
        DYLD_INSERT_LIBRARIES="$SHIM_LIB" \
        DYLD_FORCE_FLAT_NAMESPACE=1 \
        "$@"
}

# ============================================================================
# Prerequisites
# ============================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  Solid Mode: Cargo Build E2E — Comprehensive (17 phases)           ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"

if ! check_prerequisites; then
    echo "❌ Prerequisites not met. Build first: cargo build"
    exit 1
fi
echo "  Shim: $SHIM_LIB"
echo ""

# ============================================================================
# Phase 0: Setup — Create a real Rust project
# ============================================================================
echo "═══ Phase 0: Setup ═══"
setup_test_workspace
cd "$TEST_WORKSPACE"

cat > Cargo.toml << 'EOF'
[package]
name = "hello-vrift"
version = "0.1.0"
edition = "2021"

[lib]
name = "hello_vrift"
path = "src/lib.rs"

[[bin]]
name = "hello-vrift"
path = "src/main.rs"
EOF

mkdir -p src
cat > src/lib.rs << 'EOF'
pub fn greet(name: &str) -> String {
    format!("Hello, {}! VeloRift v1.", name)
}

pub fn compute(x: i32) -> i32 {
    x * 2
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() { assert!(greet("World").contains("v1")); }
    #[test]
    fn test_compute() { assert_eq!(compute(21), 42); }
}
EOF

cat > src/main.rs << 'EOF'
fn main() {
    println!("{}", hello_vrift::greet("World"));
    println!("compute(21) = {}", hello_vrift::compute(21));
}
EOF

pass "Created Rust project"

# ============================================================================
# Phase 1: Baseline build (no inception)
# ============================================================================
echo ""
echo "═══ Phase 1: Baseline build (no inception) ═══"

BUILD_OUT=$(cargo build 2>&1) && pass "Baseline build" || { fail "Baseline build"; echo "$BUILD_OUT"; exit 1; }
assert_output "Output: v1 greeting" "VeloRift v1" ./target/debug/hello-vrift
assert_output "Output: compute=42" "compute(21) = 42" ./target/debug/hello-vrift

TEST_OUT=$(cargo test 2>&1)
echo "$TEST_OUT" | grep -q "test result: ok" && pass "Baseline test" || fail "Baseline test"

# ============================================================================
# Phase 2: Ingest → CAS + VDir
# ============================================================================
echo ""
echo "═══ Phase 2: Ingest ═══"

start_daemon "warn"
sleep 1

INGEST_OUT=$(VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
    "$VRIFT_CLI" ingest --parallel . 2>&1) && pass "Ingest" || { fail "Ingest"; echo "$INGEST_OUT"; exit 1; }

sleep 2

VDIR_MMAP_PATH=""
[ -d "$TEST_WORKSPACE/.vrift/vdir" ] && VDIR_MMAP_PATH=$(find "$TEST_WORKSPACE/.vrift/vdir" -name "*.vdir" 2>/dev/null | head -1)
[ -z "$VDIR_MMAP_PATH" ] && VDIR_MMAP_PATH=$(find "${HOME}/.vrift/vdir" -name "*.vdir" -newer "$TEST_WORKSPACE/Cargo.toml" 2>/dev/null | head -1)
[ -n "$VDIR_MMAP_PATH" ] && pass "VDir found" || { fail "VDir not found"; exit 1; }

# ============================================================================
# Phase 3: Inception build (initial)
# ============================================================================
echo ""
echo "═══ Phase 3: Inception build ═══"

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Inception build" || { fail "Inception build"; echo "$BUILD_OUT"; }
assert_output "v1 under inception" "VeloRift v1" INCEP ./target/debug/hello-vrift
assert_output "compute under inception" "compute(21) = 42" INCEP ./target/debug/hello-vrift

# ============================================================================
# Phase 4: Real code change — v1 → v2
# ============================================================================
echo ""
echo "═══ Phase 4: Code change (v1→v2) ═══"

cat > src/lib.rs << 'EOF'
pub fn greet(name: &str) -> String {
    format!("Hello, {}! VeloRift v2.", name)
}

pub fn compute(x: i32) -> i32 {
    x * 3
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() { assert!(greet("World").contains("v2")); }
    #[test]
    fn test_compute() { assert_eq!(compute(21), 63); }
}
EOF

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Build after v2 change" || fail "Build after v2 change"
assert_recompiled "$BUILD_OUT" "hello-vrift"
assert_output "Now shows v2" "VeloRift v2" INCEP ./target/debug/hello-vrift
assert_output "Compute now 63" "compute(21) = 63" INCEP ./target/debug/hello-vrift
assert_no_output "No stale v1" "VeloRift v1" INCEP ./target/debug/hello-vrift

# ============================================================================
# Phase 5: Compile error → fix → rebuild
# ============================================================================
echo ""
echo "═══ Phase 5: Compile error → fix → rebuild ═══"

# Inject syntax error
cat > src/lib.rs << 'EOF'
pub fn greet(name: &str) -> String {
    format!("Hello, {}! VeloRift v2.", name)
    // INTENTIONAL ERROR: missing semicolon after extra statement
    let broken = 42
}

pub fn compute(x: i32) -> i32 { x * 3 }
EOF

BUILD_ERR=$(INCEP cargo build 2>&1) || true
if [ $? -ne 0 ] || echo "$BUILD_ERR" | grep -q "error"; then
    pass "Broken code fails to compile"
else
    fail "Broken code should fail to compile"
fi

# Verify error message is useful (shows location)
if echo "$BUILD_ERR" | grep -q "lib.rs"; then
    pass "Error points to lib.rs"
else
    fail "Error message doesn't mention lib.rs"
fi

# Fix the error
cat > src/lib.rs << 'EOF'
pub fn greet(name: &str) -> String {
    format!("Hello, {}! VeloRift v2-fixed.", name)
}

pub fn compute(x: i32) -> i32 { x * 3 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() { assert!(greet("World").contains("v2-fixed")); }
    #[test]
    fn test_compute() { assert_eq!(compute(21), 63); }
}
EOF

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Build after fix" || fail "Build after fix"
assert_output "Shows v2-fixed" "v2-fixed" INCEP ./target/debug/hello-vrift

# ============================================================================
# Phase 6: cargo check + cargo run
# ============================================================================
echo ""
echo "═══ Phase 6: cargo check + cargo run ═══"

# cargo check: fast type-check, no binary
CHECK_OUT=$(INCEP cargo check 2>&1) && pass "cargo check" || fail "cargo check"

# cargo run: build + execute in one step
RUN_OUT=$(INCEP cargo run 2>&1)
if echo "$RUN_OUT" | grep -q "v2-fixed"; then
    pass "cargo run output correct"
else
    fail "cargo run output wrong: $(echo "$RUN_OUT" | tail -1)"
fi

# ============================================================================
# Phase 7: Add new module
# ============================================================================
echo ""
echo "═══ Phase 7: Add new module ═══"

cat > src/utils.rs << 'EOF'
pub fn reverse(s: &str) -> String {
    s.chars().rev().collect()
}

pub fn uppercase(s: &str) -> String {
    s.to_uppercase()
}
EOF

cat > src/lib.rs << 'EOF'
pub mod utils;

pub fn greet(name: &str) -> String {
    let rev = utils::reverse(name);
    let up = utils::uppercase(name);
    format!("Hello {}! (rev={}, up={}) VeloRift v3.", name, rev, up)
}

pub fn compute(x: i32) -> i32 { x * 3 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() {
        let out = greet("World");
        assert!(out.contains("v3") && out.contains("dlroW") && out.contains("WORLD"));
    }
    #[test]
    fn test_reverse() { assert_eq!(utils::reverse("abc"), "cba"); }
    #[test]
    fn test_uppercase() { assert_eq!(utils::uppercase("hello"), "HELLO"); }
    #[test]
    fn test_compute() { assert_eq!(compute(21), 63); }
}
EOF

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Build with new module" || fail "Build with new module"
assert_output "Shows reversed" "dlroW" INCEP ./target/debug/hello-vrift
assert_output "Shows uppercase" "WORLD" INCEP ./target/debug/hello-vrift
assert_output "Shows v3" "VeloRift v3" INCEP ./target/debug/hello-vrift

# ============================================================================
# Phase 8: build.rs — build script generates code
# ============================================================================
echo ""
echo "═══ Phase 8: build.rs (build script) ═══"

cat > build.rs << 'EOF'
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("generated.rs");
    fs::write(&dest, "pub const BUILD_TAG: &str = \"built-by-vrift-e2e\";\n").unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
EOF

# Update lib.rs to include generated code
cat > src/lib.rs << 'EOF'
pub mod utils;
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub fn greet(name: &str) -> String {
    let rev = utils::reverse(name);
    format!("Hello {}! (rev={}, tag={}) VeloRift v4.", name, rev, generated::BUILD_TAG)
}

pub fn compute(x: i32) -> i32 { x * 3 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() { assert!(greet("World").contains("v4")); }
    #[test]
    fn test_build_tag() { assert_eq!(generated::BUILD_TAG, "built-by-vrift-e2e"); }
    #[test]
    fn test_compute() { assert_eq!(compute(21), 63); }
}
EOF

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Build with build.rs" || { fail "Build with build.rs"; echo "$BUILD_OUT"; }
assert_output "Shows build tag" "built-by-vrift-e2e" INCEP ./target/debug/hello-vrift
assert_output "Shows v4" "VeloRift v4" INCEP ./target/debug/hello-vrift

# Verify OUT_DIR files were created
OUT_DIR=$(find "$TEST_WORKSPACE/target/debug/build/hello-vrift-"*/out -name "generated.rs" 2>/dev/null | head -1)
if [ -n "$OUT_DIR" ]; then
    pass "build.rs generated file exists"
else
    fail "build.rs generated file not found"
fi

# ============================================================================
# Phase 9: Delete source file (refactor)
# ============================================================================
echo ""
echo "═══ Phase 9: Delete source file ═══"

rm -f src/utils.rs

# Update lib.rs to remove utils dependency
cat > src/lib.rs << 'EOF'
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub fn greet(name: &str) -> String {
    format!("Hello {}! (tag={}) VeloRift v5-no-utils.", name, generated::BUILD_TAG)
}

pub fn compute(x: i32) -> i32 { x * 3 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() { assert!(greet("World").contains("v5-no-utils")); }
    #[test]
    fn test_compute() { assert_eq!(compute(21), 63); }
}
EOF

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Build after file deletion" || { fail "Build after file deletion"; echo "$BUILD_OUT"; }
assert_output "Shows v5-no-utils" "v5-no-utils" INCEP ./target/debug/hello-vrift
assert_no_output "No reverse (utils removed)" "dlroW" INCEP ./target/debug/hello-vrift

# Verify src/utils.rs is truly gone
if [ ! -f src/utils.rs ]; then
    pass "utils.rs deleted from filesystem"
else
    fail "utils.rs still exists"
fi

# ============================================================================
# Phase 10: Add dependency (Cargo.toml change)
# ============================================================================
echo ""
echo "═══ Phase 10: Add dependency ═══"

# Use a small, common crate
cat > Cargo.toml << 'EOF'
[package]
name = "hello-vrift"
version = "0.1.0"
edition = "2021"

[dependencies]
cfg-if = "1"

[lib]
name = "hello_vrift"
path = "src/lib.rs"

[[bin]]
name = "hello-vrift"
path = "src/main.rs"
EOF

cat > src/lib.rs << 'EOF'
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub fn greet(name: &str) -> String {
    // Use cfg-if to prove dependency works
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            let platform = "unix";
        } else {
            let platform = "other";
        }
    }
    format!("Hello {}! platform={} VeloRift v6.", name, platform)
}

pub fn compute(x: i32) -> i32 { x * 3 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() { assert!(greet("World").contains("v6")); }
    #[test]
    fn test_platform() { assert!(greet("X").contains("unix")); }
}
EOF

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Build with new dependency" || { fail "Build with new dep"; echo "$BUILD_OUT"; }
assert_output "Shows v6" "VeloRift v6" INCEP ./target/debug/hello-vrift
assert_output "Shows platform" "platform=unix" INCEP ./target/debug/hello-vrift

# ============================================================================
# Phase 11: Revert change (undo modification)
# ============================================================================
echo ""
echo "═══ Phase 11: Revert change ═══"

# Revert to simpler code (remove cfg-if dependency)
cat > Cargo.toml << 'EOF'
[package]
name = "hello-vrift"
version = "0.1.0"
edition = "2021"

[lib]
name = "hello_vrift"
path = "src/lib.rs"

[[bin]]
name = "hello-vrift"
path = "src/main.rs"
EOF

# Revert lib.rs to a previous-like state
cat > src/lib.rs << 'EOF'
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub fn greet(name: &str) -> String {
    format!("Hello {}! (tag={}) VeloRift v7-reverted.", name, generated::BUILD_TAG)
}

pub fn compute(x: i32) -> i32 { x * 2 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_greet() { assert!(greet("World").contains("v7-reverted")); }
    #[test]
    fn test_compute() { assert_eq!(compute(21), 42); }
}
EOF

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Build after revert" || { fail "Build after revert"; echo "$BUILD_OUT"; }
assert_recompiled "$BUILD_OUT" "hello-vrift" "Revert triggers recompile"
assert_output "Shows v7-reverted" "v7-reverted" INCEP ./target/debug/hello-vrift
assert_output "Compute back to 42" "compute(21) = 42" INCEP ./target/debug/hello-vrift
assert_no_output "No v6 (reverted)" "VeloRift v6" INCEP ./target/debug/hello-vrift

# ============================================================================
# Phase 12: cargo test under inception
# ============================================================================
echo ""
echo "═══ Phase 12: cargo test ═══"

TEST_OUT=$(INCEP cargo test 2>&1)
if echo "$TEST_OUT" | grep -q "test result: ok"; then
    pass "cargo test under inception"
    echo "  $(echo "$TEST_OUT" | grep 'test result' | grep -o '[0-9]* passed' | head -1)"
else
    fail "cargo test under inception"
    echo "$TEST_OUT" | tail -5
fi

if echo "$TEST_OUT" | grep -q "test_greet"; then
    pass "test_greet executed"
else
    fail "test_greet not found"
fi

# ============================================================================
# Phase 13: Clean → Rebuild from CAS
# ============================================================================
echo ""
echo "═══ Phase 13: Clean rebuild from CAS ═══"

chflags -R nouchg "$TEST_WORKSPACE/target" 2>/dev/null || true
rm -rf "$TEST_WORKSPACE/target"
[ ! -d "$TEST_WORKSPACE/target" ] && pass "target/ removed" || fail "target/ exists"

BUILD_OUT=$(INCEP cargo build 2>&1) && pass "Clean rebuild" || { fail "Clean rebuild"; echo "$BUILD_OUT"; }
assert_output "After rebuild: v7-reverted" "v7-reverted" INCEP ./target/debug/hello-vrift
assert_output "After rebuild: compute=42" "compute(21) = 42" INCEP ./target/debug/hello-vrift
assert_real_file "$TEST_WORKSPACE/target/debug/hello-vrift" "Binary is real file"
assert_no_uchg "$TEST_WORKSPACE/target/debug/hello-vrift" "Binary no uchg"

# Check a few rlib files too
for rlib in $(find "$TEST_WORKSPACE/target/debug/deps" -name "*.rlib" 2>/dev/null | head -3); do
    assert_no_uchg "$rlib" "rlib no uchg"
done

# ============================================================================
# Phase 14: Post-inception (no shim)
# ============================================================================
echo ""
echo "═══ Phase 14: Post-inception build ═══"

cargo build 2>&1 && pass "Post-inception build" || fail "Post-inception build"
assert_output "Works without inception" "v7-reverted" ./target/debug/hello-vrift
assert_output "Compute without inception" "compute(21) = 42" ./target/debug/hello-vrift

TEST_OUT=$(cargo test 2>&1)
echo "$TEST_OUT" | grep -q "test result: ok" && pass "Post-inception test" || fail "Post-inception test"

# ============================================================================
# Phase 15: CAS integrity
# ============================================================================
echo ""
echo "═══ Phase 15: CAS integrity ═══"

CAS_OK=true
CHECKED=0
for blob in $(find "$VR_THE_SOURCE" -name "*.bin" 2>/dev/null | head -20); do
    fname=$(basename "$blob")
    hash=$(echo "$fname" | cut -d_ -f1)
    if [ ${#hash} -ge 32 ] && command -v b3sum >/dev/null 2>&1; then
        actual=$(b3sum --no-names "$blob" 2>/dev/null | head -c ${#hash})
        [ "$hash" != "$actual" ] && { fail "CAS mismatch: $fname"; CAS_OK=false; }
        CHECKED=$((CHECKED + 1))
    fi
done
[ "$CAS_OK" = true ] && pass "CAS integrity: $CHECKED verified"

if [ "$(uname)" = "Darwin" ]; then
    UCHG=$(find "$VR_THE_SOURCE" -name "*.bin" -flags uchg 2>/dev/null | wc -l | tr -d ' ')
    TOTAL=$(find "$VR_THE_SOURCE" -name "*.bin" 2>/dev/null | wc -l | tr -d ' ')
    echo "  CAS uchg: $UCHG / $TOTAL blobs"
fi

# ============================================================================
# Phase 16: Benchmark timing
# ============================================================================
echo ""
echo "═══ Phase 16: Benchmark ═══"

# No-op build timing (use python3 for portability — macOS date has no %N)
ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
T_START=$(ms)
INCEP cargo build >/dev/null 2>&1
T_END=$(ms)
NOOP_MS=$((T_END - T_START))
echo "  No-op inception build: ${NOOP_MS}ms"

if [ "$NOOP_MS" -le 3000 ]; then
    pass "No-op < 3s (${NOOP_MS}ms)"
else
    fail "No-op too slow: ${NOOP_MS}ms (expected < 3s)"
fi

# ============================================================================
# Summary
# ============================================================================
stop_daemon

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS: $PASSED passed, $FAILED failed"
echo "═══════════════════════════════════════════════════════════════"

[ "$FAILED" -gt 0 ] && { echo "  ❌ SOME TESTS FAILED"; exit 1; }
echo "  ✅ ALL TESTS PASSED"
exit 0
