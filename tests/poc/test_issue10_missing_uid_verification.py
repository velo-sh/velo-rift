#!/usr/bin/env python3
"""
Test: Issue #10 - Missing Client UID/GID Verification on IPC
Expected: FAIL (Daemon accepts ManifestUpsert from any user without verification)
Fixed: SUCCESS (Daemon rejects requests for paths not owned by the caller)
"""

import os
import socket
import struct
import sys

SOCKET_PATH = "/tmp/vrift.sock"


def send_request(sock, request_bytes):
    """Send a bincode request and receive response."""
    length = struct.pack("<I", len(request_bytes))
    sock.sendall(length + request_bytes)

    resp_len_bytes = sock.recv(4)
    if len(resp_len_bytes) < 4:
        return None
    resp_len = struct.unpack("<I", resp_len_bytes)[0]
    return sock.recv(resp_len)


def main():
    print("=== Test: Missing Client UID/GID Verification on IPC ===")
    print("Issue: Daemon does not verify caller identity via SO_PEERCRED/getpeereid.")
    print("")

    # Check if daemon source has peer credential verification
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.dirname(os.path.dirname(script_dir))
    daemon_src = os.path.join(project_root, "crates/vrift-daemon/src/main.rs")

    with open(daemon_src) as f:
        content = f.read()

    checks = [
        ("SO_PEERCRED", "Linux peer credential check"),
        ("getpeereid", "BSD/macOS peer credential check"),
        ("ucred", "Unix credential struct"),
        ("peer_cred", "Tokio/Rust peer credential API"),
    ]

    found_any = False
    for pattern, desc in checks:
        if pattern in content:
            print(f"[FOUND] {desc}: '{pattern}'")
            found_any = True

    if not found_any:
        print("[FAIL] No client identity verification found in daemon!")
        print("")
        print("Security Impact:")
        print("  - Any local user can send ManifestUpsert to corrupt another user's manifest")
        print("  - Any local user can send Protect to modify file permissions")
        print("")

        # Try to connect and verify no auth needed
        if os.path.exists(SOCKET_PATH):
            print("[INFO] Attempting to connect to daemon socket...")
            try:
                sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                sock.connect(SOCKET_PATH)
                print("[CONNECTED] Socket connection accepted without authentication.")
                sock.close()
            except Exception as e:
                print(f"[INFO] Could not connect: {e}")
        else:
            print(f"[INFO] Daemon not running (socket {SOCKET_PATH} not found)")

        return 1
    else:
        print("[PASS] Daemon appears to verify client identity.")
        return 0


if __name__ == "__main__":
    sys.exit(main())
