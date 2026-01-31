#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::TempDir;
    use vrift_cas::{parallel_ingest, IngestMode};

    /// Verify "Iron Law Drift" Bug (Idempotency Bug)
    /// If a file already exists in CAS but is unprotected, re-ingest should restore protection.
    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn test_iron_law_idempotency() {
        let cas_dir = TempDir::new().unwrap();
        let source_dir = TempDir::new().unwrap();
        let cas_root = cas_dir.path();

        let file_path = source_dir.path().join("vulnerable_file.txt");
        fs::write(&file_path, "secret content").unwrap();

        // 1. Manually place file in CAS without protection (simulating legacy/corrupt state)
        let hash = "6159b27af9a3c2ca93c75be16ae492127292b623bc4aa8a4f450aed029a4407d";
        let blob_path = cas_root
            .join("blake3")
            .join("61")
            .join("59")
            .join(format!("{}_14.bin", hash));
        fs::create_dir_all(blob_path.parent().unwrap()).unwrap();
        fs::write(&blob_path, "secret content").unwrap();

        // Ensure it starts as writable
        let mut perms = fs::metadata(&blob_path).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&blob_path, perms).unwrap();

        println!(
            "Initial blob perms: {:?}",
            fs::metadata(&blob_path).unwrap().permissions()
        );

        // 2. Run standard ingest pipeline
        let files = vec![file_path.clone()];
        let results = parallel_ingest(&files, cas_root, IngestMode::SolidTier2);
        assert!(results[0].is_ok());

        // 3. Verify if CAS blob is now enforced with protection
        // If the bug exists, was_new=false will cause it to skip enforce_cas_invariant
        let metadata = fs::metadata(&blob_path).unwrap();
        assert!(
            metadata.permissions().readonly(),
            "CAS blob must be set to READ-ONLY even if it already existed"
        );
    }
}
