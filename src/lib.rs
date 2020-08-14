//! Thread-safe, shared (asynchronous) stream buffer designed to lock only on accessing and storing new data.
//!
//! Streamcatcher is designed to allow seeking on otherwise one-way streams (*e.g.*, command output)
//! whose output needs to be accessed by many threads without constant reallocations,
//! contention over safe read-only data, or unnecessary stalling. Only threads who read in
//! *new data* ever need to lock the data structure, and do not prevent earlier reads from occurring.
//!
//! # Features
//!
//! * Lockless access to pre-read data and finished streams.
//! * Transparent caching of newly read data.
//! * Allows seeking on read-only bytestreams.
//! * Piecewise allocation to reduce copying and support unknown input lengths.
//! * Optional acceleration of reads on stream completion by copying to a single backing store.
//! * (Stateful) bytestream transformations.
//!
//! The main algorithm is outlined in [this blog post], with rope
//! reference tracking moved to occur only in the core.
//!
//! # Examples
//! ```
//! use streamcatcher::Catcher;
//! use std::io::{
//!     self,
//!     Read,
//!     Seek,
//!     SeekFrom,
//! };
//!
//! const THREAD_COUNT: usize = 256;
//! const PROCESS_LEN: u64 = 10_000_000;
//!
//! // A read-only process, which many threads need to (re-)use.
//! let mut process = io::repeat(0xAC)
//!     .take(PROCESS_LEN);
//!
//! let mut catcher = Catcher::new(process);
//!
//! // Many workers who need this data...
//! let mut handles = (0..THREAD_COUNT)
//!     .map(|v| {
//!         let mut handle = catcher.new_handle();
//!         std::thread::spawn(move || {
//!             let mut buf = [0u8; 4_096];
//!             let mut correct_bytes = 0;
//!             while let Ok(count) = handle.read(&mut buf[..]) {
//!                 if count == 0 { break }
//!                 for &byte in buf[..count].iter() {
//!                     if byte == 0xAC { correct_bytes += 1 }
//!                 }
//!             }
//!             correct_bytes
//!         })
//!     })
//!     .collect::<Vec<_>>();
//!
//! // And everything read out just fine!
//! let count_correct = handles.drain(..)
//!     .map(|h| h.join().unwrap())
//!     .filter(|&v| v == PROCESS_LEN)
//!     .count();
//!
//! assert_eq!(count_correct, THREAD_COUNT);
//!
//! // Moving forwards and backwards *just works*.
//! catcher.seek(SeekFrom::End(0));
//! assert_eq!(io::copy(&mut catcher, &mut io::sink()).unwrap(), 0);
//!
//! catcher.seek(SeekFrom::Current(-256));
//! assert_eq!(io::copy(&mut catcher, &mut io::sink()).unwrap(), 256);
//!
//! ```
//!
//! [this blog post]: https://mcfelix.me/blog/shared-buffers/

#[cfg(feature = "async")]
pub mod future;
mod loom;
#[cfg(feature = "standard")]
mod standard;
mod state;

pub(crate) use crate::loom::*;
#[cfg(feature = "standard")]
pub use standard::*;
pub use state::*;

use core::result::Result as CoreResult;
use std::error::Error;

/// Shorthand for configuration error handling.
pub type Result<T> = CoreResult<T, CatcherError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The number of bytes output by a [`Transform`] into a [`TxCatcher`].
///
/// [`Transform`]: trait.Transform.html
/// [`TxCatcher`]: struct.TxCatcher.html
pub enum TransformPosition {
	/// Indication that a stream has not yet finished.
	///
	/// This has different semantics from `Read::read`. Ordinarily, `Ok(0)` denotes the end-of-file,
	/// but some transforms (*e.g.*, audio compression) need to read in enough bytes before
	/// they can output any further data, and might return `Read(0)`.
	Read(usize),

	/// Indication that a stream has definitely finished.
	Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Streamcatcher configuration errors.
pub enum CatcherError {
	/// Returned when the chunk size is smaller than a [`Transform`]'s
	/// [`minimum required contiguous byte count`].
	///
	/// [`Transform`]: trait.Transform.html
	/// [`minimum required contiguous byte count`]: trait.Transform.html#method.min_bytes_required
	ChunkSize,

	/// Returned when an async-only [`Finaliser`] is passed into a synchronous
	/// [`TxCatcher`].
	///
	/// [`Finaliser`]: enum.Finaliser.html
	/// [`TxCatcher`]: struct.TxCatcher.html
	IllegalFinaliser,
}

impl std::fmt::Display for CatcherError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{:?}", self)
	}
}

impl Error for CatcherError {}

#[derive(Clone, Debug)]
/// Method to allocate a new contiguous backing store, if required by
/// [`Config::use_backing`].
///
/// Choosing the incorrect async runtime may cause a panic, and any values other than
/// [`InPlace`] or [`NewThread`] will result in an error in a synchronous [`Catcher`].
///
/// [`Config::use_backing`]: struct.Config.html#method.use_backing
/// [`InPlace`]: #variant.InPlace
/// [`NewThread`]: #variant.NewThread
/// [`Catcher`]: type.Catcher.html
pub enum Finaliser {
	/// Allocate the new store and copy in all bytes in-place, blocking the current thread.
	InPlace,

	/// Allocate the new store and copy in all bytes in-place in a new thread.
	///
	/// *Default*, safe to call in an async context.
	NewThread,
	// #[cfg(feature = "async")]
	// Async(Box<dyn Spawn>),
	#[cfg(feature = "async-std-compat")]
	/// Use the async-std runtime for backing-store creation.
	///
	/// Requires the `"async-std-compat"` feature.
	AsyncStd,

	#[cfg(feature = "tokio-compat")]
	/// Use the tokio runtime for backing-store creation.
	///
	/// Requires the `"tokio-compat"` feature.
	Tokio,

	#[cfg(feature = "smol-compat")]
	/// Use the smol runtime for backing-store creation.
	///
	/// Requires the `"smol-compat"` feature.
	Smol,
}

impl Finaliser {
	/// Returns whether this option will block a reading thread in a sync-friendly manner.
	pub fn is_sync(&self) -> bool {
		matches!(self, Finaliser::InPlace | Finaliser::NewThread)
	}

	/// Returns whether this option will block a reading thread to finalise.
	pub fn run_elsewhere(&self) -> bool {
		!matches!(self, Finaliser::InPlace)
	}
}

#[derive(Clone, Debug)]
/// Options controlling backing store allocation, finalisation, and so on.
pub struct Config {
	chunk_size: usize,
	spawn_finaliser: Finaliser,
	use_backing: bool,
	length_hint: Option<usize>,
	read_burst_len: usize,
}

impl Config {
	pub fn new() -> Self {
		Self {
			chunk_size: 4096,
			spawn_finaliser: Finaliser::NewThread,
			use_backing: true,
			length_hint: None,
			read_burst_len: 4096,
		}
	}

	/// The amount of bytes to allocate whenever more space is required to
	/// store the stream.
	///
	/// A larger value is generally preferred for minimising locking and allocations,
	/// but may reserve too much space before the struct is finalised.
	///
	/// *Defaults to `4096`. Must be larger than the transform's minimum chunk size.*
	pub fn chunk_size(&mut self, size: usize) -> &mut Self {
		self.chunk_size = size;
		self
	}

	/// Allocate a single contiguous backing store to speed up reads after the stream ends.
	///
	/// *Defaults to `true`.*
	pub fn use_backing(&mut self, val: bool) -> &mut Self {
		self.use_backing = val;
		self
	}

	/// Spawn a new thread/task to move contents of the rope into backing storage once
	/// a stream completes.
	///
	/// Disabling this may negatively impact performance of the final read in a stream.
	///
	/// *Defaults to [`Finaliser::NewThread`].*
	///
	/// [`Finaliser::NewThread`]: enum.FInaliser.html#variant.NewThread
	pub fn spawn_finaliser(&mut self, finaliser: Finaliser) -> &mut Self {
		self.spawn_finaliser = finaliser;
		self
	}

	/// Estimate for the amount of data required to store the completed stream.
	///
	/// On `None`, this will be set to [`chunk_size`].
	///
	/// *Defaults to `None`.*
	///
	/// [`chunk_size`]: #method.chunk_size
	pub fn length_hint(&mut self, hint: Option<usize>) -> &mut Self {
		self.length_hint = hint;
		self
	}
}

impl Default for Config {
	fn default() -> Self {
		Self::new()
	}
}

#[derive(Clone, Copy, Debug, Default)]
/// A no-op data transform.
pub struct Identity {}

#[derive(Debug)]
struct BufferChunk {
	data: Vec<u8>,

	start_pos: usize,
	end_pos: usize,
}

impl BufferChunk {
	fn new(start_pos: usize, chunk_len: usize) -> Self {
		BufferChunk {
			data: Vec::with_capacity(chunk_len),

			start_pos,
			end_pos: start_pos,
		}
	}
}

trait RopeAndState {
	const SHIFT_AMT: usize;
	const HOLD_FLAGS: usize = !(0b11 << Self::SHIFT_AMT);

	fn state(self) -> FinaliseState;
	fn upgrade_state(self) -> Self;

	fn holders(self) -> Holders<Self>
	where
		Self: Sized;
}

impl RopeAndState for usize {
	const SHIFT_AMT: usize = (usize::MAX.count_ones() as usize) - 2;

	fn state(self) -> FinaliseState {
		FinaliseState::from(self >> Self::SHIFT_AMT)
	}

	fn upgrade_state(self) -> Self {
		self + (1 << Self::SHIFT_AMT)
	}

	fn holders(self) -> Holders<Self> {
		Holders(self & Self::HOLD_FLAGS)
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Holders<T>(T);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheReadLocation {
	Roped,
	Backed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinaliseState {
	Live,
	Finalising,
	Finalised,
}

impl From<usize> for FinaliseState {
	fn from(val: usize) -> Self {
		use FinaliseState::*;
		match val {
			0 => Live,
			1 => Finalising,
			2 => Finalised,
			_ => unreachable!(),
		}
	}
}

impl From<FinaliseState> for usize {
	fn from(val: FinaliseState) -> Self {
		use FinaliseState::*;
		match val {
			Live => 0,
			Finalising => 1,
			Finalised => 2,
		}
	}
}

impl FinaliseState {
	fn is_source_live(self) -> bool {
		matches!(self, FinaliseState::Live)
	}

	fn is_source_finished(self) -> bool {
		!self.is_source_live()
	}

	fn is_backing_ready(self) -> bool {
		matches!(self, FinaliseState::Finalised)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn state_upgrade() {
		const INIT_USERS: usize =
			0b0000_0000_0000_0000_0000_0000_0000_0000_0000_0010_0000_0100_0000_0000_0000_0011;

		const UPGRADE_1: usize =
			0b0100_0000_0000_0000_0000_0000_0000_0000_0000_0010_0000_0100_0000_0000_0000_0011;
		const UPGRADE_2: usize =
			0b1000_0000_0000_0000_0000_0000_0000_0000_0000_0010_0000_0100_0000_0000_0000_0011;

		let u1 = INIT_USERS.upgrade_state();
		let u2 = u1.upgrade_state();

		assert_eq!(u1, UPGRADE_1);
		assert_eq!(u2, UPGRADE_2);

		assert_eq!(INIT_USERS.state(), FinaliseState::Live);
		assert_eq!(u1.state(), FinaliseState::Finalising);
		assert_eq!(u2.state(), FinaliseState::Finalised);

		assert_eq!(INIT_USERS.holders().0, INIT_USERS);
		assert_eq!(u1.holders().0, INIT_USERS);
		assert_eq!(u2.holders().0, INIT_USERS);
	}
}
