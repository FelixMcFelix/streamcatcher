#[cfg(loom)]
mod safety {
	use loom::{
		cell::UnsafeCell,
		sync::{Arc, Notify},
		thread,
	};
	use std::io::{Read, Seek, SeekFrom};
	use streamcatcher::*;

	#[test]
	fn create() {
		const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

		loom::model(|| {
			let catcher = Catcher::new(&INPUT[..]);
		});
	}

	#[test]
	fn identity() {
		const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

		loom::model(|| {
			let catcher = Catcher::new(&INPUT[..]);

			let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

			assert_eq!(&out[..], &INPUT[..]);
		});
	}

	#[test]
	fn two_accessors() {
		loom::model(|| {
			let mut perma_array = Box::new(vec![]);
			for i in 0..8 {
				perma_array.push(i as u8);
			}
			let input: &'static _ = Box::leak(perma_array);

			let mut cfg = Config::new();
			cfg.spawn_finaliser(Finaliser::InPlace);

			let catcher = Catcher::with_config(&input[..], cfg).unwrap();

			let mut handles = Vec::with_capacity(2);

			for _i in 0..2 {
				let mut h = catcher.clone();
				handles.push(thread::spawn(move || {
					let mut buf = [0u8; 4];
					while let Ok(n) = h.read(&mut buf[..]) {
						if n == 0 {
							break;
						}
					}
				}));
			}

			for handle in handles.drain(..) {
				handle.join().unwrap()
			}
		});
	}

	#[test]
	fn three_accessors() {
		loom::model(|| {
			let mut perma_array = Box::new(vec![]);
			for i in 0..4096 {
				perma_array.push(i as u8);
			}
			let input: &'static _ = Box::leak(perma_array);

			let mut cfg = Config::new();
			cfg.spawn_finaliser(Finaliser::InPlace);

			let catcher = Catcher::with_config(&input[..], cfg).unwrap();

			let mut handles = Vec::with_capacity(3);

			for _i in 0..3 {
				let mut h = catcher.clone();
				handles.push(thread::spawn(move || {
					let mut buf = [0u8; 1024];
					while let Ok(n) = h.read(&mut buf[..]) {
						if n == 0 {
							break;
						}
					}
				}));
			}

			for handle in handles.drain(..) {
				handle.join().unwrap()
			}
		});
	}

	// #[test]
	// fn will_loom_allow() {
	// 	loom::model(|| {
	// 		let x = Arc::new(UnsafeCell::new(14));
	// 		let w = Arc::new(Notify::new());

	// 		let x_c = x.clone();
	// 		let w_c = w.clone();
	// 		let a = thread::spawn(move || {
	// 			// w_c.wait();
	// 			x_c.with(|a| {});
	// 		});

	// 		let x_c = x.clone();
	// 		let w_c = w.clone();
	// 		let b = thread::spawn(move || {
	// 			// w_c.wait();
	// 			x_c.with(|a| {});
	// 		});

	// 		w.notify();

	// 		a.join();
	// 		b.join();
	// 	});
	// }
}

#[cfg(not(loom))]
mod no_loom {
	#[test]
	fn loom_cfg_flag_disabled() {
		// To run Loom tests, set RUSTFLAGS="--cfg loom"
		// or $env:RUSTFLAGS = "--cfg loom" on Windows.
	}
}
