use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tempfile::TempDir;
use vrift_cas::CasStore;

fn bench_cas_store(c: &mut Criterion) {
    let temp = TempDir::new().unwrap();
    let cas = CasStore::new(temp.path()).unwrap();
    let data = vec![0u8; 1024 * 10]; // 10KB

    c.bench_function("cas_store_10kb", |b| {
        b.iter(|| {
            // Note: This will repeatedly store the same content (deduplication path)
            // To test write speed, we might need unique content, but for CAS `store` checks,
            // deduplication is a valid/common path.
            // Let's vary usage if needed, but for now strict store overhead is fine.
            cas.store(black_box(&data)).unwrap()
        })
    });
}

fn bench_cas_get(c: &mut Criterion) {
    let temp = TempDir::new().unwrap();
    let cas = CasStore::new(temp.path()).unwrap();
    let data = vec![0u8; 1024 * 10]; // 10KB
    let hash = cas.store(&data).unwrap();

    c.bench_function("cas_get_10kb", |b| {
        b.iter(|| cas.get(black_box(&hash)).unwrap())
    });
}

fn bench_cas_get_mmap(c: &mut Criterion) {
    let temp = TempDir::new().unwrap();
    let cas = CasStore::new(temp.path()).unwrap();
    let data = vec![0u8; 1024 * 1024]; // 1MB
    let hash = cas.store(&data).unwrap();

    c.bench_function("cas_get_mmap_1mb", |b| {
        b.iter(|| cas.get_mmap(black_box(&hash)).unwrap())
    });
}

criterion_group!(benches, bench_cas_store, bench_cas_get, bench_cas_get_mmap);
criterion_main!(benches);
