#[cfg(feature = "smol-compat")]
mod async_std {
	use futures::io::{
		AsyncRead,
		AsyncReadExt,
	};
	use streamcatcher::future::*;

	#[test]
	fn identity() {
		const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

		smol::run(async {
			let mut catcher = Catcher::new(&INPUT[..], None).unwrap();
			let mut space = vec![0u8; INPUT.len()];
			let mut read_in = 0;

			while read_in < space.len() {
				match catcher.read(&mut space[read_in..]).await {
					Ok(n) => read_in += n,
					Err(e) => panic!("{:?}", e),
				}
			}

			assert_eq!(read_in, INPUT.len());
			assert_eq!(space[..], INPUT[..]);
		});
	}
}

#[cfg(feature = "tokio-compat")]
mod tokio {
	use futures::io::{
		AsyncRead,
		AsyncReadExt,
	};
	use streamcatcher::future::*;
	use tokio::runtime::Builder;

	#[test]
	fn identity() {
		const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

		smol::run(async {
			unimplemented!()
		});
	}
}
