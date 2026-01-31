#!/usr/bin/env python3
"""
VRift E2E Regression Test Suite

Comprehensive end-to-end tests for zero-copy ingest functionality.
Tests tiered datasets, dedup, EPERM handling, and cross-project dedup.

Usage:
    uv run python scripts/e2e_test.py
    # or
    python3 scripts/e2e_test.py

Requirements:
    - Python 3.11+
    - Node.js / npm (for dependency installation)
    - Built vrift binary (cargo build --release)
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

# ============================================================================
# Configuration
# ============================================================================

SCRIPT_DIR = Path(__file__).parent.absolute()
PROJECT_ROOT = SCRIPT_DIR.parent
VRIFT_BINARY = PROJECT_ROOT / "target" / "release" / "vrift"
BENCHMARKS_DIR = PROJECT_ROOT / "examples" / "benchmarks"

# Test datasets
DATASETS = {
    "small": {
        "package": "small_package.json",
        "min_files": 10000,
        "max_time_sec": 10,
    },
    "medium": {
        "package": "medium_package.json",
        "min_files": 20000,
        "max_time_sec": 15,
    },
    "large": {
        "package": "large_package.json",
        "min_files": 50000,
        "max_time_sec": 30,
    },
    "xlarge": {
        "package": "xlarge_package.json",
        "min_files": 100000,
        "max_time_sec": 60,
    },
}


@dataclass
class TestResult:
    name: str
    passed: bool
    duration_sec: float
    files: int
    message: str


# ============================================================================
# Utilities
# ============================================================================

def run_cmd(cmd: list[str], cwd: Optional[Path] = None, timeout: int = 600) -> tuple[int, str, str]:
    """Run a command and return (exit_code, stdout, stderr)."""
    try:
        result = subprocess.run(
            cmd,
            cwd=cwd,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "Timeout"


def count_files(directory: Path) -> int:
    """Count files in directory recursively."""
    count = 0
    for _, _, files in os.walk(directory):
        count += len(files)
    return count


def get_dir_size_mb(directory: Path) -> float:
    """Get directory size in MB."""
    total = 0
    for entry in directory.rglob("*"):
        if entry.is_file():
            total += entry.stat().st_size
    return total / (1024 * 1024)


def print_result(result: TestResult):
    """Print test result with color."""
    status = "✅ PASS" if result.passed else "❌ FAIL"
    print(f"  {status} {result.name}")
    print(f"       Files: {result.files:,} | Time: {result.duration_sec:.2f}s | {result.message}")


# ============================================================================
# Test Cases
# ============================================================================

def test_binary_build() -> TestResult:
    """Test 1: Ensure vrift binary is built."""
    start = time.time()
    
    if not VRIFT_BINARY.exists():
        # Try to build
        code, _, stderr = run_cmd(
            ["cargo", "build", "--release", "-p", "vrift-cli"],
            cwd=PROJECT_ROOT,
            timeout=300,
        )
        if code != 0:
            return TestResult(
                name="Binary Build",
                passed=False,
                duration_sec=time.time() - start,
                files=0,
                message=f"Build failed: {stderr[:100]}",
            )
    
    # Verify binary works
    code, stdout, _ = run_cmd([str(VRIFT_BINARY), "--version"])
    
    return TestResult(
        name="Binary Build",
        passed=code == 0,
        duration_sec=time.time() - start,
        files=0,
        message=stdout.strip() if code == 0 else "Binary not working",
    )


def test_dataset_ingest(name: str, config: dict, work_dir: Path, cas_dir: Path) -> TestResult:
    """Test: Ingest a dataset and verify."""
    start = time.time()
    
    package_json = BENCHMARKS_DIR / config["package"]
    if not package_json.exists():
        return TestResult(
            name=f"Ingest {name}",
            passed=False,
            duration_sec=0,
            files=0,
            message=f"Package not found: {package_json}",
        )
    
    # Setup work directory
    dataset_dir = work_dir / name
    dataset_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy(package_json, dataset_dir / "package.json")
    
    # Install dependencies
    install_start = time.time()
    code, _, stderr = run_cmd(
        ["npm", "install", "--legacy-peer-deps", "--silent"],
        cwd=dataset_dir,
        timeout=300,
    )
    if code != 0:
        # Try without legacy-peer-deps
        code, _, stderr = run_cmd(
            ["npm", "install", "--silent"],
            cwd=dataset_dir,
            timeout=300,
        )
    
    if code != 0:
        return TestResult(
            name=f"Ingest {name}",
            passed=False,
            duration_sec=time.time() - start,
            files=0,
            message=f"npm install failed: {stderr[:100]}",
        )
    
    install_time = time.time() - install_start
    
    # Count files
    node_modules = dataset_dir / "node_modules"
    file_count = count_files(node_modules)
    
    if file_count < config["min_files"]:
        return TestResult(
            name=f"Ingest {name}",
            passed=False,
            duration_sec=time.time() - start,
            files=file_count,
            message=f"Too few files: {file_count} < {config['min_files']}",
        )
    
    # Clear any previous vrift metadata
    vrift_meta = node_modules / ".vrift"
    if vrift_meta.exists():
        shutil.rmtree(vrift_meta)
    
    # Run ingest
    manifest_path = work_dir / f"{name}_manifest.bin"
    ingest_start = time.time()
    code, stdout, stderr = run_cmd(
        [str(VRIFT_BINARY), "--cas-root", str(cas_dir), "ingest", str(node_modules), "-o", str(manifest_path)],
        timeout=config["max_time_sec"] * 2,
    )
    ingest_time = time.time() - ingest_start
    
    if code != 0:
        return TestResult(
            name=f"Ingest {name}",
            passed=False,
            duration_sec=time.time() - start,
            files=file_count,
            message=f"Ingest failed: {stderr[:200]}",
        )
    
    # Verify manifest created
    if not manifest_path.exists():
        return TestResult(
            name=f"Ingest {name}",
            passed=False,
            duration_sec=time.time() - start,
            files=file_count,
            message="Manifest not created",
        )
    
    # Check timing
    passed = ingest_time <= config["max_time_sec"]
    rate = int(file_count / ingest_time) if ingest_time > 0 else 0
    
    return TestResult(
        name=f"Ingest {name}",
        passed=passed,
        duration_sec=ingest_time,
        files=file_count,
        message=f"{rate:,} files/sec (npm: {install_time:.1f}s)" + ("" if passed else f" [SLOW: max {config['max_time_sec']}s]"),
    )


def test_dedup_efficiency(work_dir: Path, cas_dir: Path) -> TestResult:
    """Test: Cross-project deduplication."""
    start = time.time()
    
    # Check CAS stats
    cas_blobs = count_files(cas_dir)
    cas_size_mb = get_dir_size_mb(cas_dir)
    
    # Count total files in all datasets
    total_files = 0
    for name in DATASETS:
        node_modules = work_dir / name / "node_modules"
        if node_modules.exists():
            total_files += count_files(node_modules)
    
    if total_files == 0:
        return TestResult(
            name="Dedup Efficiency",
            passed=False,
            duration_sec=time.time() - start,
            files=0,
            message="No files ingested",
        )
    
    dedup_ratio = 1 - (cas_blobs / total_files) if total_files > 0 else 0
    passed = dedup_ratio > 0.1  # At least 10% dedup
    
    return TestResult(
        name="Dedup Efficiency",
        passed=passed,
        duration_sec=time.time() - start,
        files=cas_blobs,
        message=f"{total_files:,} files → {cas_blobs:,} blobs ({dedup_ratio*100:.1f}% dedup, {cas_size_mb:.0f}MB)",
    )


def test_eperm_handling(work_dir: Path, cas_dir: Path) -> TestResult:
    """Test: EPERM handling for code-signed bundles (requires puppeteer)."""
    start = time.time()
    
    # Check if xlarge was ingested (contains puppeteer)
    xlarge_dir = work_dir / "xlarge" / "node_modules"
    if not xlarge_dir.exists():
        return TestResult(
            name="EPERM Handling",
            passed=True,  # Skip if xlarge not tested
            duration_sec=0,
            files=0,
            message="Skipped (xlarge not tested)",
        )
    
    # Look for Chromium.app
    chromium_paths = list(xlarge_dir.rglob("Chromium.app"))
    has_chromium = len(chromium_paths) > 0
    
    # Check if ingest succeeded (xlarge result would show success)
    manifest = work_dir / "xlarge_manifest.bin"
    ingest_succeeded = manifest.exists()
    
    return TestResult(
        name="EPERM Handling",
        passed=ingest_succeeded,
        duration_sec=time.time() - start,
        files=len(chromium_paths),
        message=f"Chromium.app found: {has_chromium}, Ingest: {'✓' if ingest_succeeded else '✗'}",
    )


def test_link_strategy() -> TestResult:
    """Test: LinkStrategy unit tests."""
    start = time.time()
    
    code, stdout, stderr = run_cmd(
        ["cargo", "test", "--package", "vrift-cas", "link_strategy", "--", "--nocapture"],
        cwd=PROJECT_ROOT,
        timeout=120,
    )
    
    # Parse test count
    passed_count = stdout.count("ok")
    
    return TestResult(
        name="LinkStrategy Tests",
        passed=code == 0,
        duration_sec=time.time() - start,
        files=0,
        message=f"{passed_count} assertions passed" if code == 0 else stderr[:100],
    )


# ============================================================================
# Main
# ============================================================================

def main():
    print("=" * 60)
    print("VRift E2E Regression Test Suite")
    print("=" * 60)
    print()
    
    # Check Python version
    if sys.version_info < (3, 10):
        print(f"❌ Python 3.10+ required, got {sys.version}")
        sys.exit(1)
    
    results: list[TestResult] = []
    
    # Test 1: Binary build
    print("📦 Test: Binary Build")
    result = test_binary_build()
    print_result(result)
    results.append(result)
    
    if not result.passed:
        print("\n❌ Cannot continue without binary")
        sys.exit(1)
    
    # Test 2: Unit tests
    print("\n🧪 Test: Unit Tests")
    result = test_link_strategy()
    print_result(result)
    results.append(result)
    
    # Create temp directories
    with tempfile.TemporaryDirectory(prefix="vrift-e2e-") as tmp:
        work_dir = Path(tmp) / "work"
        cas_dir = Path(tmp) / "cas"
        work_dir.mkdir()
        cas_dir.mkdir()
        
        print(f"\n📁 Work dir: {work_dir}")
        print(f"📁 CAS dir: {cas_dir}")
        
        # Test 3-6: Dataset ingests (share same CAS for dedup testing)
        datasets_to_test = ["small", "medium"]  # Fast tests
        if "--full" in sys.argv:
            datasets_to_test = ["small", "medium", "large", "xlarge"]
        
        for name in datasets_to_test:
            print(f"\n📊 Test: Ingest {name.upper()}")
            config = DATASETS[name]
            result = test_dataset_ingest(name, config, work_dir, cas_dir)
            print_result(result)
            results.append(result)
        
        # Test: Dedup efficiency
        print("\n🔗 Test: Dedup Efficiency")
        result = test_dedup_efficiency(work_dir, cas_dir)
        print_result(result)
        results.append(result)
        
        # Test: EPERM handling
        if "xlarge" in datasets_to_test:
            print("\n🍎 Test: EPERM Handling (macOS)")
            result = test_eperm_handling(work_dir, cas_dir)
            print_result(result)
            results.append(result)
    
    # Summary
    print("\n" + "=" * 60)
    passed = sum(1 for r in results if r.passed)
    total = len(results)
    
    if passed == total:
        print(f"✅ ALL TESTS PASSED ({passed}/{total})")
        sys.exit(0)
    else:
        print(f"❌ TESTS FAILED ({passed}/{total})")
        for r in results:
            if not r.passed:
                print(f"   - {r.name}: {r.message}")
        sys.exit(1)


if __name__ == "__main__":
    main()
