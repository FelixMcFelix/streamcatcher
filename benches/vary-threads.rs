use criterion::{black_box, criterion_group, criterion_main, Bencher, BenchmarkId, Criterion};
use std::{
	io::Read,
	sync::Arc,
	thread::{self, JoinHandle},
	time::Duration,
};
use streamcatcher::{Catcher, Config, Finaliser, ReadSkipExt};
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
	let mut perma_array = Box::new(vec![]);
	for i in 0..10_000_000 {
		perma_array.push(i as u8);
	}
	let input: &'static _ = Box::leak(perma_array);

	let mut group = c.benchmark_group("Concurrent Reader Count (10MB, in-memory)");
	for i in (1..102).step_by(20) {
		group.bench_with_input(BenchmarkId::new("Naive", i), &i, move |b, i| {
			let src = black_box(NaiveSharedRead::new(&input[..]));
			read_to_end(b, *i, src);
		});
		group.bench_with_input(BenchmarkId::new("Streamcatcher", i), &i, move |b, i| {
			let src = black_box(Catcher::new(&input[..]));
			read_to_end(b, *i, src);
		});
	}

	group.finish();
}

pub fn size_known(c: &mut Criterion) {
	let mut perma_array = Box::new(vec![]);
	for i in 0..10_000_000 {
		perma_array.push(i as u8);
	}
	let input: &'static _ = Box::leak(perma_array);

	let cfg = Config::new().length_hint(Some(10_000_000));

	let mut group =
		c.benchmark_group("Concurrent Reader Count (10MB, in-memory, size hint correct)");
	for i in (1..102).step_by(20) {
		let c = cfg.clone();
		group.bench_with_input(BenchmarkId::new("Naive", i), &i, move |b, i| {
			let src = black_box(NaiveSharedRead::new(&input[..]));
			read_to_end(b, *i, src);
		});
		group.bench_with_input(BenchmarkId::new("Streamcatcher", i), &i, |b, i| {
			let c = cfg.clone();
			let src = black_box(c.build(&input[..]).unwrap());
			read_to_end(b, *i, src);
		});
	}

	group.finish();
}

pub fn prefinalised(c: &mut Criterion) {
	let mut perma_array = Box::new(vec![]);
	for i in 0..10_000_000 {
		perma_array.push(i as u8);
	}
	let input: &'static _ = Box::leak(perma_array);

	let shared_naive = NaiveSharedRead::new(&input[..]);

	let shared_catcher = Config::new()
		.spawn_finaliser(Finaliser::InPlace)
		.build(&input[..])
		.unwrap();

	shared_catcher.clone().skip(10_000_000);
	shared_naive.clone().skip(10_000_000);

	let mut group = c.benchmark_group("Concurrent Reader Count (10MB, in-memory, pre-read)");
	for i in (1..102).step_by(20) {
		group.bench_with_input(BenchmarkId::new("Naive", i), &i, |b, i| {
			let src = shared_naive.clone();
			read_to_end(b, *i, src);
		});
		group.bench_with_input(BenchmarkId::new("Streamcatcher", i), &i, |b, i| {
			let src = shared_catcher.clone();
			read_to_end(b, *i, src);
		});
	}

	group.finish();
}

criterion_group!(benches, size_known, prefinalised, default);
criterion_main!(benches);
