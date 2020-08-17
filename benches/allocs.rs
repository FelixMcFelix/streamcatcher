use criterion::{black_box, criterion_group, criterion_main, Bencher, BenchmarkId, Criterion};
use std::{
	io::Read,
	sync::Arc,
	thread::{self, JoinHandle},
	time::Duration,
};
use streamcatcher::{Catcher, Config, Finaliser, GrowthStrategy, ReadSkipExt};
use synchronoise::SignalEvent;
use utils::NaiveSharedRead;

fn prep_threads<T>(source: T, threads: usize) -> (Vec<JoinHandle<()>>, Arc<SignalEvent>)
where
	T: Read + Clone + Send + 'static,
{
	let mut handles = Vec::with_capacity(threads);
	let sig = Arc::new(SignalEvent::manual(false));

	for _i in 0..threads {
		let mut h = source.clone();
		let s = sig.clone();
		handles.push(thread::spawn(move || {
			let mut buf = [0u8; 1024];
			s.wait();
			while let Ok(n) = h.read(&mut buf[..]) {
				if n == 0 {
					break;
				}
			}
		}));
	}

	(handles, sig)
}

fn read_to_end<T>(b: &mut Bencher, i: usize, src: T)
where
	T: Read + Clone + Send + 'static,
{
	let (mut handles, sig) = prep_threads(src, i);

	b.iter(move || {
		sig.signal();
		for handle in handles.drain(..) {
			handle.join().unwrap()
		}
	});
}

pub fn default(c: &mut Criterion) {
	const THREADS: usize = 101;

	let mut perma_array = Box::new(vec![]);
	for i in 0..10_000_000 {
		perma_array.push(i as u8);
	}
	let input: &'static _ = Box::leak(perma_array);

	let mut group = c.benchmark_group("Allocation Strategy (10MB, in-memory, 101 threads)");

	group.bench_function(BenchmarkId::from_parameter("Constant(4096)"), move |b| {
		let src = black_box(Catcher::new(&input[..]));
		read_to_end(b, THREADS, src);
	});

	group.bench_function(
		BenchmarkId::from_parameter("Linear(4096, 64MB)"),
		move |b| {
			let src = black_box(
				Config::new()
					.chunk_size(GrowthStrategy::Linear {
						start: 4096,
						max: 64 * 1024 * 1024,
					})
					.build(&input[..])
					.unwrap(),
			);
			read_to_end(b, THREADS, src);
		},
	);

	group.bench_function(
		BenchmarkId::from_parameter("Geometric(4096, 64MB)"),
		move |b| {
			let src = black_box(
				Config::new()
					.chunk_size(GrowthStrategy::Geometric {
						start: 4096,
						max: 64 * 1024 * 1024,
					})
					.build(&input[..])
					.unwrap(),
			);
			read_to_end(b, THREADS, src);
		},
	);

	group.finish();
}

criterion_group!(benches, default);
criterion_main!(benches);
