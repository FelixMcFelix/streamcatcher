#[cfg(feature = "tokio-compat")]
use ::tokio::io::{AsyncRead as TokioRead, AsyncSeek as TokioSeek};
use futures::io::{AsyncRead, AsyncReadExt, Cursor};
use streamcatcher::{future::*, Config, Finaliser, Result};

async fn identity() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut catcher = Catcher::new(&INPUT[..], None).unwrap();
	let mut space = vec![0u8; INPUT.len()];
	let mut read_in = 0;

	while read_in < space.len() {
		match catcher.read(&mut space[read_in..]).await {
			Ok(n) => read_in += n,
			Err(e) => panic!("{:?}", e),
		}
	}

	assert!(catcher.is_finalised());

	assert_eq!(read_in, INPUT.len());
	assert_eq!(space[..], INPUT[..]);
}

async fn spawn_finalise(fin: Finaliser) {
	let mut input = vec![];
	for i in 0u32..(1 << 20) {
		input.push(i as u8);
	}

	let mut cfg = Config::new();

	cfg.spawn_finaliser(fin);

	let mut catcher = Catcher::new(Cursor::new(input.clone()), None).unwrap();
	let mut space = vec![0u8; input.len() + 1];
	let mut read_in = 0;

	while read_in < input.len() {
		match catcher.read(&mut space[read_in..]).await {
			Ok(n) => read_in += n,
			Err(e) => panic!("{:?}", e),
		}
	}

	assert!(catcher.is_finalised());

	assert_eq!(read_in, input.len());
	assert_eq!(space[..input.len()], input[..]);
}

#[cfg(feature = "async-std-compat")]
mod async_std {
	use streamcatcher::Finaliser;

	#[async_std::test]
	async fn identity() {
		super::identity().await;
	}

	#[async_std::test]
	async fn spawn_finalise() {
		super::spawn_finalise(Finaliser::AsyncStd).await;
	}
}

#[cfg(feature = "smol-compat")]
mod smol {
	use streamcatcher::Finaliser;

	#[test]
	fn identity() {
		smol::run(async { super::identity().await });
	}

	#[test]
	fn spawn_finalise() {
		smol::run(async { super::spawn_finalise(Finaliser::Smol).await });
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
	async fn spawn_finalise() {
		super::spawn_finalise(Finaliser::Tokio).await;
	}
}
