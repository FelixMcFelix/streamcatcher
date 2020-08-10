use std::io::{Read, Seek, SeekFrom};
use streamcatcher::*;

#[test]
fn identity() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let catcher = Catcher::new(&INPUT[..]);

	let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out[..], INPUT[..]);
}

#[test]
fn finalises() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut cfg = Config::new();
	cfg.spawn_finaliser(Finaliser::InPlace);

	let mut catcher = Catcher::with_config(&INPUT[..], cfg).unwrap();

	catcher.load_all();

	assert!(catcher.is_finalised());
}

#[test]
fn seek_start() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut catcher = Catcher::new(&INPUT[..]);

	let _ = catcher
		.new_handle()
		.bytes()
		.map(|x| x.unwrap())
		.collect::<Vec<_>>();

	const START: usize = 3;

	let pos = catcher.seek(SeekFrom::Start(START as u64));

	let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out[..], INPUT[START..]);
	assert!(pos.is_ok());
	if let Ok(start) = pos {
		assert_eq!(start as usize, START);
	}
}

#[test]
fn seek_end() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut catcher = Catcher::new(&INPUT[..]);

	let _ = catcher
		.new_handle()
		.bytes()
		.map(|x| x.unwrap())
		.collect::<Vec<_>>();

	const END: usize = 3;

	let pos = catcher.seek(SeekFrom::End(-(END as i64)));

	let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out[..], INPUT[INPUT.len() - END..]);
	assert!(pos.is_ok());
	if let Ok(start) = pos {
		assert_eq!(start as usize, INPUT.len() - END);
	}
}

#[test]
fn seek_current() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let mut catcher = Catcher::new(&INPUT[..]);

	let _ = catcher
		.new_handle()
		.bytes()
		.map(|x| x.unwrap())
		.collect::<Vec<_>>();

	const START: usize = 6;
	const SHIFT: i64 = -2;

	let pos = catcher.seek(SeekFrom::Start(START as u64));
	let pos = catcher.seek(SeekFrom::Current(SHIFT));

	let out = catcher
		.clone()
		.bytes()
		.map(|x| x.unwrap())
		.collect::<Vec<_>>();

	assert_eq!(out[..], INPUT[((START as i64) + SHIFT) as usize..]);
	assert!(pos.is_ok());
	if let Ok(start) = pos {
		assert_eq!(start as usize, ((START as i64) + SHIFT) as usize);
	}
}

#[test]
fn read_after_complete() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let catcher = Catcher::new(&INPUT[..]);
	let catcher_clone = catcher.new_handle();

	let out_1 = catcher_clone
		.bytes()
		.map(|x| x.unwrap())
		.collect::<Vec<_>>();
	let out_2 = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out_1[..], INPUT[..]);
	assert_eq!(out_2[..], INPUT[..]);
}

#[test]
fn shared_access() {
	// unimplemented!()
}
