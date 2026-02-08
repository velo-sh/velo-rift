#!/usr/bin/env python3
"""
VRift Performance Benchmark Suite

Comprehensive benchmarks for VRift ingest, measuring:
- Deduplication efficiency (THE key value)
- Space savings from content-addressable storage
- Re-ingest speed (incremental performance)
- Cross-project dedup (monorepo scenarios)

Usage:
    python3 scripts/benchmark_suite.py             # Full benchmark
    python3 scripts/benchmark_suite.py --quick     # Quick mode (small/medium only)
    python3 scripts/benchmark_suite.py --report    # Generate markdown report only
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path

# ============================================================================
# Configuration
# ============================================================================

SCRIPT_DIR = Path(__file__).parent.absolute()
PROJECT_ROOT = SCRIPT_DIR.parent
VRIFT_BINARY = PROJECT_ROOT / "target" / "release" / "vrift"
BENCHMARKS_DIR = PROJECT_ROOT / "examples" / "benchmarks"
REPORT_DIR = PROJECT_ROOT / "docs"

DATASETS = {
    "xsmall": {"package": "xsmall_package.json", "tier": "quick"},
    "small": {"package": "small_package.json", "tier": "quick"},
    "medium": {"package": "medium_package.json", "tier": "full"},
    "large": {"package": "large_package.json", "tier": "full"},
    "xxlarge": {"package": "xxlarge_package.json", "tier": "full"},
}


@dataclass
class BenchmarkResult:
    """Single benchmark run result."""

    name: str
    files: int
    bytes_processed: int
    duration_sec: float
    unique_blobs: int = 0
    memory_peak_mb: float = 0.0

    @property
    def files_per_sec(self) -> float:
        return self.files / self.duration_sec if self.duration_sec > 0 else 0

    @property
    def mb_per_sec(self) -> float:
        return (self.bytes_processed / 1024 / 1024) / self.duration_sec if self.duration_sec > 0 else 0

    @property
    def dedup_ratio(self) -> float:
        return 1 - (self.unique_blobs / self.files) if self.files > 0 else 0


@dataclass
class BenchmarkSuite:
    """Collection of benchmark results."""

    results: list[BenchmarkResult] = field(default_factory=list)
    timestamp: str = field(default_factory=lambda: datetime.now().isoformat())

    def add(self, result: BenchmarkResult) -> None:
        self.results.append(result)

    def get(self, name: str) -> BenchmarkResult | None:
        return next((r for r in self.results if r.name == name), None)


# ============================================================================
# Utilities
# ============================================================================


def run_cmd(cmd: list[str], cwd: Path | None = None, timeout: int = 600) -> tuple[int, str, str]:
    """Run command and return (code, stdout, stderr)."""
    try:
        result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "Timeout"


def count_files(directory: Path) -> int:
    """Count files recursively."""
    count = 0
    for _, _, files in os.walk(directory):
        count += len(files)
    return count


def rmtree_onerror(func, path, exc_info):
    """Handler for shutil.rmtree to skip permission errors during cleanup."""
    import stat

    if not os.access(path, os.W_OK):
        # Try to make it writable
        try:
            os.chmod(path, stat.S_IWUSR)
            func(path)
        except Exception:
            pass  # Skip if we still can't delete it (likely VFS protected)


def get_dir_size(directory: Path) -> int:
    """Get directory size in bytes."""
    total = 0
    for entry in directory.rglob("*"):
        if entry.is_file():
            total += entry.stat().st_size
    return total


def format_bytes(bytes_val: float) -> str:
    """Human-readable bytes."""
    for unit in ["B", "KB", "MB", "GB"]:
        if bytes_val < 1024:
            return f"{bytes_val:.1f} {unit}"
        bytes_val /= 1024
    return f"{bytes_val:.1f} TB"


def format_number(n: int) -> str:
    """Format with commas."""
    return f"{n:,}"


# ============================================================================
# Benchmark Functions
# ============================================================================


def benchmark_vrift_ingest(
    source_dir: Path,
    cas_dir: Path,
    manifest_path: Path,
) -> BenchmarkResult:
    """Benchmark VRift ingest."""
    files = count_files(source_dir)
    size = get_dir_size(source_dir)

    # Clear any previous metadata
    vrift_meta = source_dir / ".vrift"
    if vrift_meta.exists():
        shutil.rmtree(vrift_meta, onerror=rmtree_onerror)

    start = time.time()
    code, stdout, stderr = run_cmd(
        [
            str(VRIFT_BINARY),
            "--the-source-root",
            str(cas_dir),
            "ingest",
            str(source_dir),
            "-o",
            str(manifest_path),
        ]
    )
    duration = time.time() - start

    if code != 0:
        print(f"  ERROR: {stderr[:200]}")
        return BenchmarkResult(name="vrift", files=files, bytes_processed=size, duration_sec=0)

    # Parse unique blobs and actual ingest duration from output
    unique_blobs = files  # fallback
    actual_duration = duration  # fallback to wall clock
    for line in stdout.split("\n"):
        if "blobs" in line and "→" in line:
            parts = line.split("→")
            if len(parts) == 2:
                blob_part = parts[1].split()[0].replace(",", "")
                try:
                    unique_blobs = int(blob_part)
                except ValueError:
                    pass
        # Parse actual ingest time: "VRift Complete in X.XXs"
        if "Complete in" in line:
            import re

            match = re.search(r"(\d+\.?\d*)\s*s", line)
            if match:
                actual_duration = float(match.group(1))

    return BenchmarkResult(
        name="vrift",
        files=files,
        bytes_processed=size,
        duration_sec=actual_duration,
        unique_blobs=unique_blobs,
    )


def run_dataset_benchmark(name: str, config: dict[str, str], work_dir: Path) -> BenchmarkSuite:
    """Run all benchmarks on a single dataset."""
    suite = BenchmarkSuite()

    package_json = BENCHMARKS_DIR / config["package"]
    if not package_json.exists():
        print(f"  SKIP: {package_json} not found")
        return suite

    dataset_dir = work_dir / name
    dataset_dir.mkdir(parents=True, exist_ok=True)

    # Install npm dependencies
    shutil.copy(package_json, dataset_dir / "package.json")
    print("  Installing npm dependencies...")
    code, _, stderr = run_cmd(
        ["npm", "install", "--legacy-peer-deps", "--silent"],
        cwd=dataset_dir,
        timeout=300,
    )
    if code != 0:
        print(f"  npm install failed: {stderr[:100]}")
        return suite

    node_modules = dataset_dir / "node_modules"
    print(f"  Dataset: {format_number(count_files(node_modules))} files, {format_bytes(get_dir_size(node_modules))}")

    # VRift benchmark
    print("  Benchmarking vrift...")
    cas_dir = work_dir / f"{name}_cas"
    cas_dir.mkdir(exist_ok=True)
    manifest = work_dir / f"{name}.manifest"
    result = benchmark_vrift_ingest(node_modules, cas_dir, manifest)
    suite.add(result)
    print(f"    {result.files_per_sec:,.0f} files/sec, {result.dedup_ratio * 100:.1f}% dedup")

    # Re-ingest benchmark (test CAS cache hit performance)
    print("  Benchmarking re-ingest (CAS hit)...")
    vrift_meta = node_modules / ".vrift"
    if vrift_meta.exists():
        shutil.rmtree(vrift_meta, onerror=rmtree_onerror)
    result2 = benchmark_vrift_ingest(node_modules, cas_dir, work_dir / f"{name}_reingest.manifest")
    result2.name = "vrift (reingest)"
    suite.add(result2)
    speedup = result.duration_sec / result2.duration_sec if result2.duration_sec > 0 else 0
    print(f"    {result2.files_per_sec:,.0f} files/sec ({speedup:.1f}x faster)")

    # CAS size analysis
    cas_size = get_dir_size(cas_dir)
    original_size = result.bytes_processed
    space_saved = original_size - cas_size
    saved_pct = 100 * space_saved / original_size if original_size > 0 else 0
    print(
        f"  Space: {format_bytes(original_size)} -> {format_bytes(cas_size)} (saved {format_bytes(space_saved)}, {saved_pct:.0f}%)"
    )

    return suite


# ============================================================================
# Report Generation
# ============================================================================


def generate_report(suites: dict[str, BenchmarkSuite]) -> str:
    """Generate markdown performance report."""
    lines = [
        "# VRift Performance Report",
        "",
        f"Generated: {datetime.now().strftime('%Y-%m-%d %H:%M')}",
        "",
        "## Key Metrics",
        "",
        "| Dataset | Files | Blobs | Dedup | Speed |",
        "|---------|-------|-------|-------|-------|",
    ]

    for name, suite in suites.items():
        vrift = suite.get("vrift")
        if vrift:
            lines.append(
                f"| {name} | {format_number(vrift.files)} | "
                f"{format_number(vrift.unique_blobs)} | "
                f"{vrift.dedup_ratio * 100:.1f}% | "
                f"{vrift.files_per_sec:,.0f}/s |"
            )

    lines.extend(
        [
            "",
            "## Deduplication Efficiency",
            "",
            "Space savings from content-addressable storage:",
            "",
        ]
    )

    for name, suite in suites.items():
        vrift = suite.get("vrift")
        if vrift:
            saved = vrift.bytes_processed * vrift.dedup_ratio
            lines.append(
                f"- **{name}**: {vrift.files:,} files -> {vrift.unique_blobs:,} blobs "
                f"({vrift.dedup_ratio * 100:.1f}% dedup, ~{format_bytes(int(saved))} saved)"
            )

    lines.extend(
        [
            "",
            "## Re-ingest Performance (CI Cache Hit)",
            "",
            "Performance when CAS already contains content:",
            "",
        ]
    )

    for name, suite in suites.items():
        first = suite.get("vrift")
        reingest = suite.get("vrift (reingest)")
        if first and reingest and first.duration_sec > 0:
            speedup = first.duration_sec / reingest.duration_sec if reingest.duration_sec > 0 else 0
            lines.append(
                f"- **{name}**: {reingest.files_per_sec:,.0f} files/sec ({speedup:.1f}x faster than first ingest)"
            )

    return "\n".join(lines)


# ============================================================================
# Main
# ============================================================================


def main() -> None:
    quick_mode = "--quick" in sys.argv
    # report_only = "--report" in sys.argv - unused

    print("╔══════════════════════════════════════════════════════════╗")
    print("║              VRift Performance Benchmark                 ║")
    print("╚══════════════════════════════════════════════════════════╝")
    print()

    # Check binary
    if not VRIFT_BINARY.exists():
        print("Building vrift...")
        code, _, stderr = run_cmd(
            ["cargo", "build", "--release", "-p", "vrift-cli"],
            cwd=PROJECT_ROOT,
        )
        if code != 0:
            print(f"Build failed: {stderr}")
            sys.exit(1)

    # Select datasets
    if quick_mode:
        datasets = {k: v for k, v in DATASETS.items() if v["tier"] == "quick"}
        print("Mode: QUICK (small + medium only)")
    else:
        datasets = DATASETS
        print("Mode: FULL (all datasets)")
    print()

    suites: dict[str, BenchmarkSuite] = {}

    try:
        with tempfile.TemporaryDirectory(prefix="vrift-bench-") as tmp:
            work_dir = Path(tmp)

            for name, config in datasets.items():
                print(f"═══ {name.upper()} ═══")
                suite = run_dataset_benchmark(name, config, work_dir)
                if suite.results:
                    suites[name] = suite
                print()
    except PermissionError:
        # Expected if VFS protected files are in the temp dir and we're on 3.10
        # The OS will eventually clean /tmp anyway
        pass

    # Generate report
    if suites:
        report = generate_report(suites)

        report_path = REPORT_DIR / "BENCHMARK.md"
        report_path.write_text(report)
        print(f"Report saved: {report_path}")

        # Print summary
        print()
        print(report)


if __name__ == "__main__":
    main()
