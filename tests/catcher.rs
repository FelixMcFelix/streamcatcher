use std::io::Read;
use streamcatcher::*;

#[test]
fn identity() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let catcher = Catcher::new(&INPUT[..], None).unwrap();

	let out = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out[..], INPUT[..]);
}

#[test]
fn read_after_complete() {
	const INPUT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

	let catcher = Catcher::new(&INPUT[..], None).unwrap();
	let catcher_clone = catcher.new_handle();

	let out_1 = catcher_clone
		.bytes()
		.map(|x| x.unwrap())
		.collect::<Vec<_>>();
	let out_2 = catcher.bytes().map(|x| x.unwrap()).collect::<Vec<_>>();

	assert_eq!(out_1[..], INPUT[..]);
	assert_eq!(out_2[..], INPUT[..]);
}
