use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use vrift_vdird::vdir::{VDir, VDirEntry};

#[test]
fn test_vdir_resize_reader_stability() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stability_test.vdir");

    // 1. Create initial VDir
    {
        let mut vdir = VDir::create_or_open(&path).unwrap();
        for i in 0..100 {
            vdir.upsert(VDirEntry {
                path_hash: i as u64 + 1000,
                ..Default::default()
            })
            .unwrap();
        }
    }

    let done = Arc::new(AtomicBool::new(false));
    let path_arc = Arc::new(path);

    // 2. Reader thread: continuously lookup
    let done_r = done.clone();
    let p_r = path_arc.clone();
    let reader = thread::spawn(move || {
        let vdir = VDir::open_readonly(&p_r).unwrap();
        let mut success = 0;
        let mut fallback = 0;
        while !done_r.load(Ordering::Relaxed) {
            // Lookup existing entry
            if vdir.lookup(1000).is_some() {
                success += 1;
            } else {
                fallback += 1;
            }
            core::hint::spin_loop();
        }
        (success, fallback)
    });

    // 3. Writer thread: trigger resize
    let mut vdir_w = VDir::create_or_open(&path_arc).unwrap();
    let initial_cap = vdir_w.get_stats().capacity;
    let target = (initial_cap as f64 * 0.77) as usize;
    for i in 0..target {
        vdir_w
            .upsert(VDirEntry {
                path_hash: i as u64 + 2000,
                ..Default::default()
            })
            .unwrap();
    }
    assert_eq!(vdir_w.get_stats().capacity, initial_cap * 2);

    done.store(true, Ordering::Relaxed);
    let (s, f) = reader.join().unwrap();

    println!("Reader: {} successes, {} fallbacks / out-of-bounds", s, f);
    // Note: In this test, the reader probably won't have "fallbacks" because its mmap
    // is fixed at original size, and it's looking up entry 1000 which is likely in the original range.
    // However, if we looked up an entry that rehashed to the NEW range, it might fail.
}
