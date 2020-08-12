#[cfg(loom)]
mod safety {
	use loom::model;
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
}

#[cfg(not(loom))]
mod no_loom {
	#[test]
	fn loom_cfg_flag_disabled() {
		// To run Loom tests, set RUSTFLAGS="--cfg loom"
		// or $env:RUSTFLAGS = "--cfg loom" on Windows.
	}
}