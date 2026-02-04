#!/usr/bin/env python3
"""
Test: Issue #11 - Protect Handler Allows Arbitrary Path Manipulation
Expected: FAIL (Protect handler accepts paths outside CAS without validation)
Fixed: SUCCESS (Protect handler rejects paths not under CAS/VFS prefix)
"""

import os
import sys


def main():
    print("=== Test: Protect Handler Arbitrary Path Manipulation ===")
    print("Issue: handle_protect accepts any path, enabling attacks on /etc/passwd etc.")
    print("")

    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.dirname(os.path.dirname(script_dir))
    daemon_src = os.path.join(project_root, "crates/vrift-daemon/src/main.rs")

    with open(daemon_src) as f:
        content = f.read()

    # Find handle_protect function
    if "async fn handle_protect" not in content:
        print("[ERROR] handle_protect function not found")
        return 1

    # Extract the function
    start = content.find("async fn handle_protect")
    end = content.find("\n}\n", start) + 3
    func_content = content[start:end]

    print("[ANALYSIS] handle_protect function:")
    print("-" * 60)

    # Check for path validation
    validations = [
        ("starts_with", "Path prefix validation"),
        ("canonicalize", "Path canonicalization"),
        ("CAS", "CAS path check"),
        ("VFS", "VFS prefix check"),
        ("vfs_prefix", "VFS prefix check"),
        ("VRIFT", "VRIFT path check"),
    ]

    found_validation = False
    for pattern, desc in validations:
        if pattern.lower() in func_content.lower():
            print(f"[FOUND] {desc}")
            found_validation = True

    if not found_validation:
        print("[FAIL] No path validation found in handle_protect!")
        print("")
        print("Security Impact:")
        print('  - Attacker can send Protect{path: "/etc/passwd", immutable: true}')
        print("  - If daemon runs as root, this would lock system files")
        print("  - Attacker can change ownership of any file via owner parameter")
        print("")
        print("Vulnerable code excerpt:")
        # Show first few lines of the function
        lines = func_content.split("\n")[:15]
        for line in lines:
            print(f"  {line}")
        return 1
    else:
        print("[PASS] handle_protect appears to validate paths.")
        return 0


if __name__ == "__main__":
    sys.exit(main())
