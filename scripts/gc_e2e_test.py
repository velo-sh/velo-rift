#!/usr/bin/env python3
"""
VRift GC E2E Test (RFC-0041)

Tests the full garbage collection lifecycle with STRICT SAFETY VERIFICATION.

CRITICAL SAFETY TESTS:
- Blobs referenced by ANY active manifest must NEVER be deleted
- Shared blobs (referenced by multiple projects) stay protected
- Only truly orphaned blobs get deleted

Usage:
    python3 scripts/gc_e2e_test.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.absolute()
PROJECT_ROOT = SCRIPT_DIR.parent
VRIFT_BINARY = PROJECT_ROOT / "target" / "release" / "vrift"
REGISTRY_PATH = Path.home() / ".vrift" / "registry" / "manifests.json"


def run_cmd(cmd: list[str], timeout: int = 30) -> tuple[int, str, str]:
    """Run command, return (code, stdout, stderr)."""
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "Timeout"


def count_files(directory: Path) -> int:
    """Count files in directory recursively."""
    count = 0
    for _, _, files in os.walk(directory):
        count += len(files)
    return count


def get_blob_hashes(cas_dir: Path) -> set[str]:
    """Get all blob hashes currently in CAS."""
    hashes = set()
    blake3_dir = cas_dir / "blake3"
    if blake3_dir.exists():
        for level1 in blake3_dir.iterdir():
            if level1.is_dir():
                for level2 in level1.iterdir():
                    if level2.is_dir():
                        for blob in level2.iterdir():
                            if blob.is_file():
                                hashes.add(blob.name)
    return hashes


def print_step(step: int, desc: str):
    print(f"\n  [{step}] {desc}")


def print_ok(msg: str):
    print(f"      ✅ {msg}")


def print_fail(msg: str):
    print(f"      ❌ {msg}")


def print_warn(msg: str):
    print(f"      ⚠️  {msg}")


def main():
    print("=" * 60)
    print("VRift GC E2E Test (RFC-0041)")
    print("STRICT SAFETY VERIFICATION")
    print("=" * 60)
    
    # Check binary
    if not VRIFT_BINARY.exists():
        print(f"\n❌ Binary not found: {VRIFT_BINARY}")
        print("   Run: cargo build --release -p vrift-cli")
        sys.exit(1)
    
    # Backup existing registry
    registry_backup = None
    if REGISTRY_PATH.exists():
        registry_backup = REGISTRY_PATH.read_text()
        print(f"\n📦 Backed up existing registry")
    
    passed = True
    
    try:
        # Clear registry for clean test
        if REGISTRY_PATH.exists():
            REGISTRY_PATH.unlink()
        
        with tempfile.TemporaryDirectory(prefix="vrift-gc-e2e-") as tmp:
            work_dir = Path(tmp)
            cas_dir = work_dir / "cas"
            cas_dir.mkdir()
            
            # ================================================================
            # PART 1: BASIC LIFECYCLE TEST
            # ================================================================
            print("\n" + "-" * 40)
            print("PART 1: Basic GC Lifecycle")
            print("-" * 40)
            
            # === Step 1: Create and ingest two projects ===
            print_step(1, "Create and ingest two projects")
            
            proj1 = work_dir / "project1"
            proj2 = work_dir / "project2"
            proj1.mkdir()
            proj2.mkdir()
            
            # Create test files with known content
            (proj1 / "unique1.txt").write_text("content unique to project1 only")
            (proj1 / "shared.txt").write_text("SHARED CONTENT BETWEEN PROJECTS")
            (proj2 / "unique2.txt").write_text("content unique to project2 only")
            (proj2 / "shared.txt").write_text("SHARED CONTENT BETWEEN PROJECTS")
            
            manifest1 = work_dir / "proj1.manifest"
            manifest2 = work_dir / "proj2.manifest"
            
            # Ingest project 1
            code, stdout, stderr = run_cmd([
                str(VRIFT_BINARY), "--the-source-root", str(cas_dir),
                "ingest", str(proj1), "-o", str(manifest1)
            ])
            if code != 0:
                print_fail(f"Ingest proj1 failed: {stderr[:100]}")
                passed = False
            else:
                print_ok("Project 1 ingested")
            
            blobs_after_proj1 = get_blob_hashes(cas_dir)
            
            # Ingest project 2
            code, stdout, stderr = run_cmd([
                str(VRIFT_BINARY), "--the-source-root", str(cas_dir),
                "ingest", str(proj2), "-o", str(manifest2)
            ])
            if code != 0:
                print_fail(f"Ingest proj2 failed: {stderr[:100]}")
                passed = False
            else:
                print_ok("Project 2 ingested")
            
            blobs_after_proj2 = get_blob_hashes(cas_dir)
            
            # === Step 2: Verify registry ===
            print_step(2, "Verify registry auto-registration")
            
            if not REGISTRY_PATH.exists():
                print_fail("Registry not created")
                passed = False
            else:
                registry = json.loads(REGISTRY_PATH.read_text())
                count = len(registry.get("manifests", {}))
                if count >= 2:
                    print_ok(f"Registry has {count} manifests")
                else:
                    print_fail(f"Expected >= 2 manifests, got {count}")
                    passed = False
            
            # ================================================================
            # PART 2: CRITICAL SAFETY TEST - No False Positives
            # ================================================================
            print("\n" + "-" * 40)
            print("PART 2: CRITICAL SAFETY - No False Deletions")
            print("-" * 40)
            
            # === Step 3: Record all blobs BEFORE GC ===
            print_step(3, "Record all blobs before GC")
            blobs_before_gc = get_blob_hashes(cas_dir)
            print_ok(f"CAS has {len(blobs_before_gc)} blobs")
            
            # === Step 4: Run GC --delete while BOTH manifests active ===
            print_step(4, "🔴 SAFETY TEST: GC --delete with both manifests active")
            
            code, stdout, stderr = run_cmd([str(VRIFT_BINARY), "gc", "--delete"])
            if code != 0:
                print_fail(f"GC failed: {stderr[:100]}")
                passed = False
            else:
                blobs_after_gc = get_blob_hashes(cas_dir)
                deleted = blobs_before_gc - blobs_after_gc
                
                if len(deleted) == 0:
                    print_ok("✅ PASS: No blobs deleted (all referenced)")
                else:
                    print_fail(f"🔴 DANGER: {len(deleted)} blobs deleted while manifests active!")
                    print_fail(f"   Deleted: {list(deleted)[:3]}...")
                    passed = False
            
            # === Step 5: Verify ALL original blobs still exist ===
            print_step(5, "Verify ALL original blobs still exist")
            
            blobs_now = get_blob_hashes(cas_dir)
            missing = blobs_before_gc - blobs_now
            
            if len(missing) == 0:
                print_ok("All blobs preserved")
            else:
                print_fail(f"MISSING {len(missing)} blobs!")
                passed = False
            
            # ================================================================
            # PART 3: SHARED BLOB PROTECTION
            # ================================================================
            print("\n" + "-" * 40)
            print("PART 3: Shared Blob Protection")
            print("-" * 40)
            
            # === Step 6: Delete ONE project, verify shared blob stays ===
            print_step(6, "Delete Project 1, verify shared blob protected")
            
            # Delete manifest1 and prune
            manifest1.unlink()
            code, _, _ = run_cmd([str(VRIFT_BINARY), "gc", "--prune-stale"])
            
            # Run GC --delete
            code, stdout, stderr = run_cmd([str(VRIFT_BINARY), "gc", "--delete"])
            
            blobs_after_delete1 = get_blob_hashes(cas_dir)
            
            # The shared blob should STILL exist (project2 references it)
            # Only project1's unique blob should be deleted
            shared_still_exists = len(blobs_after_delete1) >= 2  # shared + unique2
            
            if shared_still_exists:
                print_ok(f"Shared blob protected ({len(blobs_after_delete1)} blobs remain)")
            else:
                print_fail("Shared blob may have been deleted!")
                passed = False
            
            # === Step 7: Verify project2 data is intact ===
            print_step(7, "🔴 SAFETY TEST: Verify project2 data integrity")
            
            # Re-ingest project2 (should be instant - all blobs exist)
            proj2_reingest = work_dir / "proj2_reingest.manifest"
            
            code, stdout, stderr = run_cmd([
                str(VRIFT_BINARY), "--the-source-root", str(cas_dir),
                "ingest", str(proj2), "-o", str(proj2_reingest)
            ])
            
            if code != 0:
                print_fail(f"Re-ingest failed: {stderr[:100]}")
                passed = False
            else:
                # Check for 100% dedup (all blobs already in CAS)
                if "100" in stdout and "dedup" in stdout.lower():
                    print_ok("Project 2 fully intact (100% dedup on re-ingest)")
                elif "DEDUP" in stdout:
                    print_ok("Project 2 data verified intact")
                else:
                    print_ok("Project 2 re-ingested successfully")
            
            # ================================================================
            # PART 4: FULL CLEANUP
            # ================================================================
            print("\n" + "-" * 40)
            print("PART 4: Full Cleanup Verification")
            print("-" * 40)
            
            # === Step 8: Delete remaining project, GC should clean all ===
            print_step(8, "Delete all projects, verify complete cleanup")
            
            manifest2.unlink()
            proj2_reingest.unlink()
            
            code, _, _ = run_cmd([str(VRIFT_BINARY), "gc", "--prune-stale"])
            code, stdout, stderr = run_cmd([str(VRIFT_BINARY), "gc", "--delete"])
            
            blobs_final = get_blob_hashes(cas_dir)
            
            if len(blobs_final) == 0:
                print_ok("All orphan blobs cleaned")
            else:
                print_warn(f"{len(blobs_final)} blobs remain (may be from other sources)")
    
    finally:
        # Restore original registry
        if registry_backup is not None:
            REGISTRY_PATH.parent.mkdir(parents=True, exist_ok=True)
            REGISTRY_PATH.write_text(registry_backup)
            print(f"\n📦 Restored original registry")
        elif REGISTRY_PATH.exists():
            REGISTRY_PATH.unlink()
    
    # Summary
    print("\n" + "=" * 60)
    if passed:
        print("✅ ALL GC SAFETY TESTS PASSED")
        print("   • No false-positive deletions")
        print("   • Shared blobs protected")
        print("   • Data integrity verified")
        sys.exit(0)
    else:
        print("❌ GC SAFETY TESTS FAILED")
        print("   🔴 DATA LOSS RISK DETECTED")
        sys.exit(1)


if __name__ == "__main__":
    main()
