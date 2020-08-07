#[cfg(feature = "tokio-compat")]
use ::tokio::io::{AsyncRead as TokioRead, AsyncSeek as TokioSeek};
use futures::io::{AsyncRead, AsyncReadExt};
use streamcatcher::{future::*, Config, Finaliser, Result};

async fn identity() -> Result<()> {
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

	Ok(())
}

#[cfg(feature = "smol-compat")]
mod smol {
	use futures::io::{AsyncRead, AsyncReadExt};
	use streamcatcher::{future::*, Config, Finaliser};

	#[test]
	fn identity() {
		smol::run(async { identity() });
	}

	#[test]
	fn spawn_finalise() {
		const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

		smol::run(async {
			let mut cfg = Config::new();

			cfg.spawn_finaliser(Finaliser::Smol);

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
		});
	}
}

#[cfg(feature = "tokio-compat")]
mod tokio {
	use futures::io::{AsyncRead, AsyncReadExt};
	use streamcatcher::{future::*, Config, Finaliser};

	#[tokio::test]
	async fn identity() {
		super::identity().await;
	}
}
