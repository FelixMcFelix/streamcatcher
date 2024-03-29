#[cfg(feature = "tokio-compat")]
use ::tokio::io::{AsyncRead as TokioRead, AsyncSeek as TokioSeek};
use futures::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, Cursor};
use std::io::SeekFrom;
use streamcatcher::{future::*, *};

async fn identity() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut catcher = Catcher::new(&INPUT[..]);
	let mut space = vec![0u8; INPUT.len()];
	let mut read_in = 0;

	while read_in < space.len() {
		match catcher.read(&mut space[read_in..]).await {
			Ok(n) => read_in += n,
			Err(e) => panic!("{:?}", e),
		}
	}

	assert!(catcher.is_finished());

	assert_eq!(read_in, INPUT.len());
	assert_eq!(space[..], INPUT[..]);
}

async fn load_all() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let catcher = Catcher::new(&INPUT[..]);
	catcher.new_handle().load_all_async().await;

	assert!(catcher.is_finished());

	assert_eq!(catcher.len(), INPUT.len());
	assert_eq!(catcher.pos(), 0);
}

async fn spawn_finalise(fin: Finaliser) {
	let mut input = vec![];
	for i in 0u32..(1 << 20) {
		input.push(i as u8);
	}

	let mut catcher = Config::new()
		.spawn_finaliser(fin)
		.build(Cursor::new(input.clone()))
		.unwrap();

	let mut space = vec![0u8; input.len() + 1];
	let mut read_in = 0;

	while read_in < input.len() {
		match catcher.read(&mut space[read_in..]).await {
			Ok(n) => read_in += n,
			Err(e) => panic!("{:?}", e),
		}
	}

	assert!(catcher.is_finished());

	assert_eq!(read_in, input.len());
	assert_eq!(space[..input.len()], input[..]);
}

async fn seek_start() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut catcher = Catcher::new(&INPUT[..]);
	let mut space = vec![0u8; INPUT.len()];
	let mut read_in = 0;

	const START: usize = 3;

	let pos = catcher.seek(SeekFrom::Start(START as u64)).await;

	assert!(pos.is_ok());
	if let Ok(start) = pos {
		assert_eq!(start as usize, START);
		assert_eq!(start as usize, catcher.pos());
	}

	while read_in < INPUT.len() {
		match catcher.read(&mut space[read_in..]).await {
			Ok(0) => break,
			Ok(n) => read_in += n,
			Err(e) => panic!("{:?}", e),
		}
	}

	assert_eq!(space[..read_in], INPUT[START..]);
}

async fn seek_end() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut catcher = Catcher::new(&INPUT[..]);
	let mut space = vec![0u8; INPUT.len()];
	let mut read_in = 0;

	const END: usize = 3;

	let pos = catcher.seek(SeekFrom::End(-(END as i64))).await;

	assert!(pos.is_ok());
	if let Ok(start) = pos {
		assert_eq!(start as usize, INPUT.len() - END);
		assert_eq!(start as usize, catcher.pos());
	}

	while read_in < INPUT.len() {
		match catcher.read(&mut space[read_in..]).await {
			Ok(0) => break,
			Ok(n) => read_in += n,
			Err(e) => panic!("{:?}", e),
		}
	}

	assert_eq!(space[..read_in], INPUT[INPUT.len() - END..]);
}

async fn read_after_complete() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let catcher = Catcher::new(&INPUT[..]);
	let catcher_clone = catcher.new_handle();

	let mut space1 = vec![0u8; INPUT.len()];
	let mut space2 = vec![0u8; INPUT.len()];

	let mut spaces = vec![(&mut space1, catcher), (&mut space2, catcher_clone)];

	for (space, mut source) in spaces.drain(0..) {
		let mut read_in = 0;
		while read_in < INPUT.len() {
			match source.read(&mut space[read_in..]).await {
				Ok(0) => break,
				Ok(n) => read_in += n,
				Err(e) => panic!("{:?}", e),
			}
		}
	}

	assert_eq!(space1[..], INPUT[..]);
	assert_eq!(space2[..], INPUT[..]);
}

#[cfg(feature = "async-std-compat")]
mod async_std {
	use streamcatcher::Finaliser;

	#[async_std::test]
	async fn identity() {
		super::identity().await;
	}

	#[async_std::test]
	async fn load_all() {
		super::load_all().await;
	}

	#[async_std::test]
	async fn spawn_finalise() {
		super::spawn_finalise(Finaliser::AsyncStd).await;
	}

	#[async_std::test]
	async fn seek_start() {
		super::seek_start().await;
	}

	#[async_std::test]
	async fn seek_end() {
		super::seek_end().await;
	}

	#[async_std::test]
	async fn read_after_complete() {
		super::read_after_complete().await;
	}
}

#[cfg(feature = "smol-compat")]
mod smol {
	use streamcatcher::Finaliser;

	#[test]
	fn identity() {
		smol::block_on(async { super::identity().await });
	}

	#[test]
	fn load_all() {
		smol::block_on(async { super::load_all().await });
	}

	#[test]
	fn spawn_finalise() {
		smol::block_on(async { super::spawn_finalise(Finaliser::Smol).await });
	}

	#[test]
	fn seek_start() {
		smol::block_on(async { super::seek_start().await });
	}

	#[test]
	fn seek_end() {
		smol::block_on(async { super::seek_end().await });
	}

	#[test]
	fn read_after_complete() {
		smol::block_on(async { super::read_after_complete().await });
	}
}

#[cfg(feature = "tokio-compat")]
mod tokio {
	use streamcatcher::Finaliser;

	#[tokio::test]
	async fn identity() {
		super::identity().await;
	}

	#[tokio::test]
	async fn load_all() {
		super::load_all().await;
	}

	#[tokio::test]
	async fn spawn_finalise() {
		super::spawn_finalise(Finaliser::Tokio).await;
	}

	#[tokio::test]
	async fn seek_start() {
		super::seek_start().await;
	}

	#[tokio::test]
	async fn seek_end() {
		super::seek_end().await;
	}

	#[tokio::test]
	async fn read_after_complete() {
		super::read_after_complete().await;
	}
}
