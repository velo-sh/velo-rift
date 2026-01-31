import os
import sys
import tempfile
import hashlib

def get_hash(path):
    with open(path, 'rb') as f:
        return hashlib.blake3(f.read()).hexdigest()

def test_bbw():
    # 1. Setup environment
    prefix = os.environ.get('VRIFT_VFS_PREFIX', '/vrift')
    test_path = os.path.join(prefix, 'workspace/test_bbw_file.txt')
    
    print(f"--- BBW Verification: {test_path} ---")
    
    # Check if file exists in VFS
    if not os.path.exists(test_path):
        print(f"[FAIL] Test file {test_path} not found in VFS projection.")
        sys.exit(1)
        
    # 2. Read original content
    with open(test_path, 'r') as f:
        original_content = f.read()
    print(f"[OK] Read original content: '{original_content.strip()}'")
    
    # 3. Trigger Break-Before-Write
    new_content = "modified by bbw script\n"
    print(f"Writing new content...")
    with open(test_path, 'w') as f:
        f.write(new_content)
        
    # 4. Verify local view updated
    with open(test_path, 'r') as f:
        current_content = f.read()
        
    if current_content == new_content:
        print("[OK] Local view updated successfully.")
    else:
        print(f"[FAIL] Local view mismatch! Expected '{new_content.strip()}', got '{current_content.strip()}'")
        sys.exit(1)

    print("[SUCCESS] BBW Verification complete.")

if __name__ == "__main__":
    test_bbw()
