use std::path::Path;
use vrift_vdird::vdir::{fnv1a_hash, VDir, VDirEntry};

#[test]
fn generate_qa_vdir() {
    let path = Path::new("/tmp/qa_populated.vdir");
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let mut vdir = VDir::create_or_open(path).expect("Failed to create VDir");
    for i in 0..1000 {
        vdir.upsert(VDirEntry {
            path_hash: fnv1a_hash(&format!("file_{}", i)),
            size: i as u64,
            ..Default::default()
        })
        .expect("Failed to upsert");
    }
    vdir.flush().expect("Failed to flush");
    println!("SUCCESS: Generated populated VDir for QA at /tmp/qa_populated.vdir");
}

#[test]
fn generate_resized_qa_vdir() {
    let path = Path::new("/tmp/qa_resized.vdir");
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let mut vdir = VDir::create_or_open(path).expect("Failed to create VDir");
    let initial_cap = vdir.get_stats().capacity;
    let target = (initial_cap as f64 * 0.77) as usize;
    for i in 0..target {
        vdir.upsert(VDirEntry {
            path_hash: i as u64 + 100,
            size: i as u64,
            ..Default::default()
        })
        .expect("Failed to upsert");
    }
    let final_stats = vdir.get_stats();
    assert_eq!(final_stats.capacity, initial_cap * 2);
    vdir.flush().expect("Failed to flush");
    println!(
        "SUCCESS: Generated resized VDir for QA at /tmp/qa_resized.vdir (Capacity: {})",
        final_stats.capacity
    );
}
