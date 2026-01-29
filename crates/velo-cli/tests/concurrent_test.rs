use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;
use tempfile::TempDir;
use velo_cas::CasStore;

#[test]
fn stress_test_concurrent_access() {
    // Q6: Concurrent Access Test
    // Simulates 50 concurrent readers accessing the CAS
    const THREAD_COUNT: usize = 50;
    const ITERATIONS: usize = 100;
    
    let temp = TempDir::new().unwrap();
    let cas_dir = temp.path().join("cas");
    
    // 1. Setup CAS with some data
    let cas = Arc::new(CasStore::new(&cas_dir).unwrap());
    
    // Store a shared blob
    let content = b"shared concurrent data";
    let hash = cas.store(content).unwrap();
    
    // Store unique blobs for each thread
    let mut thread_hashes = Vec::new();
    for i in 0..THREAD_COUNT {
        let unique_content = format!("thread data {}", i);
        let h = cas.store(unique_content.as_bytes()).unwrap();
        thread_hashes.push(h);
    }
    let thread_hashes = Arc::new(thread_hashes);
    
    println!("Starting {} threads, {} iterations each...", THREAD_COUNT, ITERATIONS);
    let start = Instant::now();
    
    let barrier = Arc::new(Barrier::new(THREAD_COUNT));
    let mut handles = Vec::new();

    for i in 0..THREAD_COUNT {
        let c = cas.clone();
        let h_shared = hash.clone();
        let h_unique = thread_hashes[i].clone();
        let b = barrier.clone();
        
        handles.push(thread::spawn(move || {
            b.wait(); // Synchronize start
            
            for _ in 0..ITERATIONS {
                // Read shared
                let data1 = c.get(&h_shared).unwrap();
                assert_eq!(data1, b"shared concurrent data");
                
                // Read unique
                let data2 = c.get(&h_unique).unwrap();
                assert_eq!(data2, format!("thread data {}", i).as_bytes());
            }
        }));
    }

    // Wait for all
    for h in handles {
        h.join().unwrap();
    }
    
    println!("Concurrent test finished in {:?}", start.elapsed());
}
