use parking_lot::RwLock;
use std::{
	io::{self, Read, Result as IoResult},
	sync::Arc,
};

#[derive(Clone, Debug)]
pub struct NaiveSharedRead<T> {
	core: Arc<RwLock<ReadCore<T>>>,
	pos: usize,
}

#[derive(Clone, Debug)]
struct ReadCore<T> {
	data: Vec<u8>,
	source: Option<T>,
	finished: bool,
}

impl<T> ReadCore<T> {
	pub fn new(source: T) -> Self {
		Self {
			data: Vec::new(),
			source: Some(source),
			finished: false,
		}
	}
}

impl<T> NaiveSharedRead<T> {
	pub fn new(source: T) -> Self {
		Self {
			core: Arc::new(RwLock::new(ReadCore::new(source))),
			pos: 0,
		}
	}

	pub fn new_handle(&self) -> Self {
		Self {
			core: self.core.clone(),
			pos: 0,
		}
	}
}

impl<T: Read> Read for NaiveSharedRead<T> {
	fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
		self.read_from_pos(buf, self.pos).map(|bytes| {
			self.pos += bytes;
			bytes
		})
	}
}

impl<T: Read> NaiveSharedRead<T> {
	fn read_from_pos(&mut self, buf: &mut [u8], pos: usize) -> IoResult<usize> {
		let (mut len_lb, done) = {
			let lock = self.core.read();
			(lock.data.len(), lock.finished)
		};

		let target_read = pos + buf.len();
		let new_bytes_needed = target_read - len_lb.min(target_read);

		if !done && new_bytes_needed > 0 {
			let mut lock = self.core.write();

			let byte_src = lock.source.take();

			if let Some(src) = byte_src {
				let mut src = src.take(new_bytes_needed as u64);
				let got = io::copy(&mut src, &mut lock.data)?;

				lock.finished = got == 0;
				len_lb += got as usize;

				if got != 0 {
					lock.source = Some(src.into_inner());
				}
			}
		}

		let lock = self.core.read();
		let read_to = len_lb.min(target_read);
		let to_use = read_to - pos;
		buf[..to_use].copy_from_slice(&lock.data[pos..read_to]);

		Ok(to_use)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn functions_somewhat() {
		const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

		let mut reader = NaiveSharedRead::new(&INPUT[..]);
		let mut space = vec![0u8; INPUT.len()];
		let mut pos = 0;

		while let Ok(n) = reader.read(&mut space[pos..]) {
			if n == 0 {
				break;
			} else {
				pos += n;
			}
		}

		assert_eq!(&space[..], &INPUT[..]);
	}

	#[test]
	fn second_read() {
		const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

		let mut reader = NaiveSharedRead::new(&INPUT[..]);
		let mut reader2 = reader.new_handle();
		let mut space = vec![0u8; INPUT.len()];
		let mut pos = 0;

		while let Ok(n) = reader.read(&mut space[pos..]) {
			if n == 0 {
				break;
			} else {
				pos += n;
			}
		}

		assert_eq!(&space[..], &INPUT[..]);

		space = vec![0u8; INPUT.len()];
		pos = 0;

		while let Ok(n) = reader2.read(&mut space[pos..]) {
			if n == 0 {
				break;
			} else {
				pos += n;
			}
		}

		assert_eq!(&space[..], &INPUT[..]);
	}
}
