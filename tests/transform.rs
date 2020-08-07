use std::{
	io::{
		Read,
		Result as IoResult,
	},
	sync::atomic::{
		AtomicU64,
		Ordering,
	},
};
use streamcatcher::*;

#[test]
fn transform_plus_one() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
	const OUTPUT: [u8; 8] = [2, 3, 4, 5, 6, 7, 8, 9];

	#[derive(Default)]
	struct PlusOne {}

	impl<T: Read> Transform<T> for PlusOne {
		fn transform_read(&mut self, src: &mut T, buf: &mut [u8]) -> IoResult<TransformPosition> {
			Ok(match src.read(buf)? {
				0 => TransformPosition::Finished,
				n => {
					for i in 0..n {
						buf[i] += 1;
					}
					TransformPosition::Read(n)
				},
			})
		}
	}

	let catcher: TxCatcher<_, PlusOne> = TxCatcher::new(&INPUT[..], None).unwrap();

	let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out[..], OUTPUT[..]);
}

#[test]
fn transform_fold() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
	const OUTPUT: [u8; 4] = [3, 7, 11, 15];

	// Assumes buf_len always >= 0.
	#[derive(Default)]
	struct FoldAdd {
		waiting: Option<u8>,
	}

	impl<T: Read> Transform<T> for FoldAdd {
		fn transform_read(&mut self, src: &mut T, buf: &mut [u8]) -> IoResult<TransformPosition> {
			let mut space = [0u8; 2];

			let space_start = if let Some(val) = self.waiting {
				space[0] = val;
				1
			} else {
				0
			};

			Ok(match src.read(&mut space[space_start..])? {
				0 => TransformPosition::Finished,
				n => {
					let count = n + space_start;
					if count == 2 {
						buf[0] = space[0] + space[1];
						self.waiting = None;
						TransformPosition::Read(1)
					} else {
						self.waiting = Some(space[space_start]);
						TransformPosition::Read(0)
					}
				},
			})
		}
	}

	let catcher: TxCatcher<_, FoldAdd> = TxCatcher::new(&INPUT[..], None).unwrap();

	let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out[..], OUTPUT[..]);
}

#[test]
fn counting_state() {
	const INPUT: [u8; 8] = [1, 0, 0, 1, 0, 0, 0, 1];
	const OUTPUT_STREAM: [u8; 8] = [1, 0, 0, 1, 0, 0, 0, 1];
	const OUTPUT_COUNT: u64 = 3;

	#[derive(Default)]
	struct CountOnes {
		count: AtomicU64,
	}

	impl<T: Read> Transform<T> for CountOnes {
		fn transform_read(&mut self, src: &mut T, buf: &mut [u8]) -> IoResult<TransformPosition> {
			Ok(match src.read(buf)? {
				0 => TransformPosition::Finished,
				n => {
					let mut ones = 0;
					for i in buf[..n].iter() {
						if *i == 1 {
							ones += 1;
						}
					}
					self.count.fetch_add(ones, Ordering::Release);
					TransformPosition::Read(n)
				},
			})
		}
	}

	impl Stateful for CountOnes {
		type State = u64;
		unsafe fn state(&self) -> Self::State {
			self.count.load(Ordering::Acquire)
		}
	}

	let catcher: TxCatcher<_, CountOnes> = TxCatcher::new(&INPUT[..], None).unwrap();
	let state_catcher = catcher.new_handle();

	let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out[..], OUTPUT_STREAM[..]);
	unsafe {
		assert_eq!(state_catcher.state(), OUTPUT_COUNT);
	}
}
