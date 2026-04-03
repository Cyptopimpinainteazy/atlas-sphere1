use criterion::{criterion_group, criterion_main, Criterion};
use sha2::{Sha256, Digest};

fn bench_sha256_direct(c: &mut Criterion) {
    let data = vec![0u8; 256];
    c.bench_function("sha256-direct", |b| b.iter(|| {
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let _ = hasher.finalize_reset();
    }));
}

criterion_group!(benches, bench_sha256_direct);
criterion_main!(benches);
