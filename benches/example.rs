use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::io::Read;
use streamcatcher::Catcher;

pub fn criterion_benchmark(c: &mut Criterion) {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	c.bench_function("full monty", |b| {
		b.iter_with_large_drop(|| {
			let catcher = Catcher::new(&INPUT[..], None).unwrap();
			let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();
		})
	});
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
