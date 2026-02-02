//! Integration test for Metadata Isolation (Inode Decoupling)
//!
//! This test solidifies the "Zero-Impact Ingest" requirement:
//! Protecting the CAS must NOT affect user project files.

use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;
use vrift_cas::link_strategy::get_strategy;
use vrift_cas::protection::{is_immutable, set_immutable};

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn test_metadata_isolation_regression() {
    let dir = tempdir().expect("Failed to create temp dir");
    let source_path = dir.path().join("project_file.txt");
    let cas_path = dir.path().join("cas_blob.bin");

    // 1. Create a "user project file"
    {
        let mut f = File::create(&source_path).expect("Failed to create source");
        f.write_all(b"user source code")
            .expect("Failed to write source");
    }

    // Ensure initial state is clean
    assert!(
        !is_immutable(&source_path).unwrap(),
        "Source should not be immutable initially"
    );

    // 2. Perform Ingest (via LinkStrategy)
    let strategy = get_strategy();
    strategy
        .link_file(&source_path, &cas_path)
        .expect("Ingest (link) failed");

    // 3. Verify Inode Isolation (The "Consensus" Fix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let source_ino = fs::metadata(&source_path).unwrap().ino();
        let cas_ino = fs::metadata(&cas_path).unwrap().ino();

        println!("Source Inode: {}", source_ino);
        println!("CAS Inode:    {}", cas_ino);

        // On modern systems (APFS/Btrfs/XFS), these MUST be different for Velo to be safe
        if source_ino == cas_ino {
            panic!(
                "REGRESSION: Source and CAS share the same Inode (Hardlink detected). \
                   Applying uchg to CAS will contaminate the user's project file!"
            );
        }
    }

    // 4. Apply "Iron Law" protection to CAS
    // Simulated: vrift-daemon or cli setting the immutable flag
    match set_immutable(&cas_path, true) {
        Ok(_) => {
            println!("Successfully applied immutable flag to CAS blob");

            // 5. THE CRITICAL CHECK: Does the source project file remain UNTOUCHED?
            let source_is_protected =
                is_immutable(&source_path).expect("Failed to check source status");

            if source_is_protected {
                // Cleanup before failing
                let _ = set_immutable(&cas_path, false);
                panic!("METADATA CONTAMINATION: User project file inherited uchg flag from CAS!");
            }

            println!("SUCCESS: User project file remains clean and writable");

            // Verify writeability of the source
            let mut f = File::options()
                .append(true)
                .open(&source_path)
                .expect("Source should still be openable for append");
            f.write_all(b"\nnew local changes")
                .expect("Source should still be writable by user");

            // Cleanup
            set_immutable(&cas_path, false).expect("Failed to unset immutable for cleanup");
        }
        Err(e) => {
            // If we lack permissions (e.g. Linux non-root), we at least verified the Inode isolation above
            println!("Skipping immutable check due to OS permissions: {}. Inode decoupling was verified anyway.", e);
        }
    }
}
